//! Remote input for compositors without a remote desktop portal (labwc and
//! other wlroots compositors): a backend for `xdg-desktop-portal`'s
//! `RemoteDesktop` portal.
//!
//! Apps such as KDE Connect (the phone as touchpad and keyboard) ask the
//! portal for a remote desktop session and send their input over libei.
//! This backend takes calls only from the portal frontend
//! (`org.freedesktop.portal.Desktop`) and approves sessions only for the
//! apps in [`ALLOWED_APPS`]; every other app is refused. A TV has nobody at
//! a keyboard to ask, so there is no dialog. It takes the libei
//! connection (an EIS server, [`reis`]) and hands the input to the
//! compositor through a virtual pointer and virtual keyboards
//! ([`output`]). Screen sharing is not offered.
//!
//! `xdg-desktop-portal` finds it by the files the image installs:
//! `rusttv.portal` (the D-Bus name and interface), a D-Bus service file
//! that starts `rusttv-portal`, and `labwc-portals.conf` choosing it for
//! `org.freedesktop.impl.portal.RemoteDesktop`.

mod output;

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use calloop::channel::{self, Channel, Sender};
use calloop::generic::Generic;
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use reis::calloop::{EisRequestSource, EisRequestSourceEvent};
use reis::eis::device::DeviceType;
use reis::eis::{self, button::ButtonState, keyboard::KeyState};
use reis::request::{Connection, Device, DeviceCapability, EisRequest};
use zbus::object_server::ObjectServer;
use zbus::zvariant::{self, ObjectPath, OwnedValue, Value};

use output::Output;

/// The backend's D-Bus name; `rusttv.portal` names it too.
const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.rusttv";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";

/// Apps that get remote input, by the app id the portal frontend gives:
/// KDE Connect (the phone as touchpad and keyboard), installed from Debian
/// or from Flathub.
const ALLOWED_APPS: [&str; 2] = ["org.kde.kdeconnect.daemon", "org.kde.kdeconnect"];

/// The portal frontend's D-Bus name: the only caller this backend serves.
const FRONTEND: &str = "org.freedesktop.portal.Desktop";

/// Whether `app_id` may have a remote desktop session.
fn allowed(app_id: &str) -> bool {
    ALLOWED_APPS.contains(&app_id)
}

/// Whether the call came from the portal frontend, so the app id in it was
/// set by the frontend and not by the app.
async fn from_frontend(header: &zbus::message::Header<'_>, connection: &zbus::Connection) -> bool {
    let Some(sender) = header.sender() else {
        return false;
    };
    let Ok(bus) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(FRONTEND) else {
        return false;
    };
    bus.get_name_owner(name)
        .await
        .is_ok_and(|owner| owner.as_str() == sender.as_str())
}

/// Whether `session` is one this backend approved and still has open.
async fn approved(server: &ObjectServer, session: &ObjectPath<'_>) -> bool {
    server.interface::<_, Session>(session).await.is_ok()
}

/// The response for a refused request.
fn refused() -> Response {
    (2, HashMap::new())
}

/// Device types of the portal: keyboard, pointer and touchscreen. Touch is
/// granted so that apps asking for all three get a session, but no touch
/// device is offered (the compositor has no virtual touchscreen).
const DEVICE_TYPES: u32 = 1 | 2 | 4;

/// What the D-Bus side asks of the input loop.
#[derive(Debug)]
enum Command {
    /// A session's app connected: serve EIS on this socket.
    Connect { session: String, socket: UnixStream },
    /// A session ended: drop its EIS connections.
    Close { session: String },
}

/// Runs the portal backend until the session bus or the compositor goes
/// away.
pub fn run() -> Result<(), String> {
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().map_err(|error| error.to_string())?;
    let handle = event_loop.handle();
    let output = Output::connect()?;
    let fd = output
        .connection
        .backend()
        .poll_fd()
        .try_clone_to_owned()
        .map_err(|error| error.to_string())?;
    handle
        .insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, state: &mut State| {
                state.output.dispatch().map_err(std::io::Error::other)?;
                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| error.to_string())?;
    let (sender, commands): (Sender<Command>, Channel<Command>) = channel::channel();
    handle
        .insert_source(commands, |event, _, state: &mut State| {
            if let channel::Event::Msg(command) = event {
                state.command(command);
            }
        })
        .map_err(|error| error.to_string())?;

    // Last, so that the portal only calls once input can go somewhere.
    let _bus = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.name(BUS_NAME))
        .and_then(|builder| {
            builder.serve_at(
                OBJECT_PATH,
                RemoteDesktop {
                    commands: Mutex::new(sender),
                },
            )
        })
        .and_then(zbus::blocking::connection::Builder::build)
        .map_err(|error| format!("session bus: {error}"))?;
    log::info!("remote desktop portal ready");

    let mut state = State {
        handle,
        output,
        connections: Vec::new(),
        next_id: 0,
    };
    loop {
        state.output.flush();
        event_loop
            .dispatch(None, &mut state)
            .map_err(|error| error.to_string())?;
    }
}

/// The input loop's state.
struct State {
    handle: LoopHandle<'static, State>,
    output: Output,
    /// EIS connections: id, session and loop source.
    connections: Vec<(u64, String, RegistrationToken)>,
    next_id: u64,
}

impl State {
    fn command(&mut self, command: Command) {
        match command {
            Command::Connect { session, socket } => self.connect(session, socket),
            Command::Close { session } => {
                let handle = self.handle.clone();
                self.connections.retain(|(_, of, token)| {
                    let keep = *of != session;
                    if !keep {
                        handle.remove(*token);
                    }
                    keep
                });
            }
        }
    }

    fn connect(&mut self, session: String, socket: UnixStream) {
        let context = match eis::Context::new(socket) {
            Ok(context) => context,
            Err(error) => {
                log::warn!("EIS connection: {error}");
                return;
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut devices = Devices::default();
        let inserted = self.handle.insert_source(
            EisRequestSource::new(context, 1),
            move |event, connection, state: &mut State| {
                let action = match event {
                    Ok(EisRequestSourceEvent::Connected) => {
                        devices.connected(connection, &state.output);
                        PostAction::Continue
                    }
                    Ok(EisRequestSourceEvent::Request(request)) => {
                        devices.request(request, &state.output)
                    }
                    Err(error) => {
                        log::info!("EIS client: {error}");
                        PostAction::Remove
                    }
                };
                let _ = connection.flush();
                state.output.flush();
                if action == PostAction::Remove {
                    state.connections.retain(|(known, _, _)| *known != id);
                }
                Ok(action)
            },
        );
        match inserted {
            Ok(token) => {
                log::info!("remote input for {session}");
                self.connections.push((id, session, token));
            }
            Err(error) => log::warn!("EIS connection: {error}"),
        }
    }
}

/// One app's EIS connection: the devices it was given.
#[derive(Default)]
struct Devices {
    keyboard: bool,
    pointer: bool,
    absolute: bool,
    text: bool,
    /// Kept alive with the connection.
    seat: Option<reis::request::Seat>,
    added: Vec<Device>,
}

impl Devices {
    /// The handshake is done: offer a seat with everything the compositor
    /// can take.
    fn connected(&mut self, connection: &Connection, output: &Output) {
        let mut capabilities = DeviceCapability::Pointer
            | DeviceCapability::Button
            | DeviceCapability::Scroll
            | DeviceCapability::Text;
        if output.devices.keymap().is_some() {
            capabilities |= DeviceCapability::Keyboard;
        }
        if output.devices.size().is_some() {
            capabilities |= DeviceCapability::PointerAbsolute;
        }
        self.seat = Some(connection.add_seat(Some("default"), capabilities));
    }

    fn request(&mut self, request: EisRequest, output: &Output) -> PostAction {
        let devices = &output.devices;
        match request {
            EisRequest::Disconnect => return PostAction::Remove,
            EisRequest::Bind(bind) => {
                let wants = |capability| bind.capabilities.contains(capability);
                if !self.pointer && wants(DeviceCapability::Pointer) {
                    self.pointer = true;
                    self.add(
                        &bind.seat,
                        "pointer",
                        DeviceCapability::Pointer
                            | DeviceCapability::Button
                            | DeviceCapability::Scroll,
                        |_| {},
                    );
                }
                if !self.absolute
                    && wants(DeviceCapability::PointerAbsolute)
                    && let Some((width, height)) = devices.size()
                {
                    self.absolute = true;
                    self.add(
                        &bind.seat,
                        "pointer-absolute",
                        DeviceCapability::PointerAbsolute
                            | DeviceCapability::Button
                            | DeviceCapability::Scroll,
                        |device| device.device().region(0, 0, width, height, 1.0),
                    );
                }
                if !self.keyboard
                    && wants(DeviceCapability::Keyboard)
                    && let Some((keymap, size)) = devices.keymap()
                {
                    self.keyboard = true;
                    self.add(
                        &bind.seat,
                        "keyboard",
                        DeviceCapability::Keyboard.into(),
                        |device| {
                            if let Some(keyboard) = device.interface::<eis::Keyboard>() {
                                keyboard.keymap(eis::keyboard::KeymapType::Xkb, size, keymap);
                            }
                        },
                    );
                }
                if !self.text && wants(DeviceCapability::Text) {
                    self.text = true;
                    self.add(&bind.seat, "text", DeviceCapability::Text.into(), |_| {});
                }
            }
            EisRequest::PointerMotion(motion) => devices.motion(motion.time, motion.dx, motion.dy),
            EisRequest::PointerMotionAbsolute(motion) => {
                devices.motion_absolute(motion.time, motion.dx_absolute, motion.dy_absolute);
            }
            EisRequest::Button(button) => devices.button(
                button.time,
                button.button,
                button.state == ButtonState::Press,
            ),
            EisRequest::ScrollDelta(scroll) => devices.scroll(scroll.time, scroll.dx, scroll.dy),
            EisRequest::ScrollStop(stop) => devices.scroll_stop(stop.time, stop.x, stop.y),
            EisRequest::ScrollCancel(stop) => devices.scroll_stop(stop.time, stop.x, stop.y),
            EisRequest::ScrollDiscrete(scroll) => {
                devices.scroll_discrete(scroll.time, scroll.discrete_dx, scroll.discrete_dy);
            }
            EisRequest::Frame(frame) => {
                if frame.device.has_capability(DeviceCapability::Pointer)
                    || frame
                        .device
                        .has_capability(DeviceCapability::PointerAbsolute)
                {
                    devices.frame();
                }
            }
            EisRequest::KeyboardKey(key) => {
                devices.key(key.time, key.key, key.state == KeyState::Press);
            }
            EisRequest::TextUtf8(text) => devices.type_text(text.time, &text.text),
            _ => {}
        }
        PostAction::Continue
    }

    fn add(
        &mut self,
        seat: &reis::request::Seat,
        name: &str,
        capabilities: enumflags2::BitFlags<DeviceCapability>,
        before_done: impl FnOnce(&Device),
    ) {
        let device = seat.add_device(Some(name), DeviceType::Virtual, capabilities, before_done);
        device.resumed();
        self.added.push(device);
    }
}

/// Results of a portal method: the response (0 = success) and its values.
type Response = (u32, HashMap<&'static str, Value<'static>>);

/// `org.freedesktop.impl.portal.RemoteDesktop`.
struct RemoteDesktop {
    commands: Mutex<Sender<Command>>,
}

impl RemoteDesktop {
    fn send(&self, command: Command) -> zbus::fdo::Result<()> {
        self.commands
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("input loop gone".into()))?
            .send(command)
            .map_err(|_| zbus::fdo::Error::Failed("input loop gone".into()))
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the portal's D-Bus methods fix the arguments; zbus adds the caller's"
)]
#[zbus::interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktop {
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Response {
        if !from_frontend(&header, connection).await {
            log::warn!("remote desktop session refused: not from the portal frontend");
            return refused();
        }
        if !allowed(app_id) {
            log::warn!("remote desktop session refused for {app_id:?}");
            return refused();
        }
        let session = Session {
            path: session_handle.to_string(),
            commands: Mutex::new(self.commands.lock().ok().map(|sender| sender.clone())),
        };
        if let Err(error) = server.at(&session_handle, session).await {
            log::warn!("session {session_handle}: {error}");
            return refused();
        }
        log::info!("remote desktop session for {app_id:?}");
        (0, HashMap::new())
    }

    async fn select_devices(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Response {
        if !from_frontend(&header, connection).await || !approved(server, &session_handle).await {
            return refused();
        }
        (0, HashMap::new())
    }

    async fn start(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Response {
        if !from_frontend(&header, connection).await || !approved(server, &session_handle).await {
            return refused();
        }
        (
            0,
            HashMap::from([
                ("devices", Value::from(DEVICE_TYPES)),
                ("clipboard_enabled", Value::from(false)),
            ]),
        )
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<zvariant::OwnedFd> {
        if !from_frontend(&header, connection).await || !approved(server, &session_handle).await {
            return Err(zbus::fdo::Error::AccessDenied(
                "no approved remote desktop session".into(),
            ));
        }
        let (ours, theirs) =
            UnixStream::pair().map_err(|error| zbus::fdo::Error::IOError(error.to_string()))?;
        self.send(Command::Connect {
            session: session_handle.to_string(),
            socket: ours,
        })?;
        Ok(zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(theirs)))
    }

    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        DEVICE_TYPES
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// `org.freedesktop.impl.portal.Session`: the portal closes it when the app
/// ends the session.
struct Session {
    path: String,
    commands: Mutex<Option<Sender<Command>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        // Only the frontend closes a session: an app could otherwise end
        // another app's (its object path is no secret).
        if !from_frontend(&header, connection).await {
            log::warn!(
                "closing {} refused: not from the portal frontend",
                self.path
            );
            return Err(zbus::fdo::Error::AccessDenied(
                "only the portal frontend closes sessions".to_owned(),
            ));
        }
        if let Ok(commands) = self.commands.lock()
            && let Some(commands) = commands.as_ref()
        {
            let _ = commands.send(Command::Close {
                session: self.path.clone(),
            });
        }
        if let Err(error) = server.remove::<Self, _>(self.path.as_str()).await {
            log::warn!("closing {}: {error}", self.path);
        }
        Ok(())
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn only_kde_connect_gets_remote_input() {
        assert!(allowed("org.kde.kdeconnect.daemon"));
        assert!(allowed("org.kde.kdeconnect"));
        for app in [
            "",
            "org.example.Keylogger",
            "org.kde.kdeconnect.daemon.evil",
        ] {
            assert!(!allowed(app), "{app}");
        }
    }
}
