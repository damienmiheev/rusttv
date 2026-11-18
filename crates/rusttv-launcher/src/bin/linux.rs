//! The home screen on Linux: the apps of this machine's desktop entries.
//!
//! Run it full screen in a Wayland compositor (e.g. labwc or cage) as the
//! session's first program. Apps it starts cover it; when an app's process
//! ends, the home screen is back.
//!
//! The tile order, hidden apps and history live in
//! `$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`).
//!
//! `RUSTTV_LANG=<tag>` overrides the language from `LC_ALL`/`LC_MESSAGES`/`LANG`.
//!
//! `rusttv-launcher-linux --home` tells the running home screen that Home was
//! pressed: it closes any page or menu and shows the tiles. Bind it to the
//! remote's Home key in the compositor, together with raising the window
//! titled `RustTV` (labwc: `ForEach` + `Raise` + `Focus` + `Execute`).
//!
//! `rusttv-launcher-linux --app-menu` does the same and then opens the menu
//! of the app that was in front (its size on screen): bind it to holding
//! Home (the remote's daemon sends `XF86Launch1` for a held Home).
//!
//! `rusttv-launcher-linux --power` does the same and then opens the power
//! menu (restart, power off): bind it to the remote's power key like Home.
//!
//! `rusttv-launcher-linux --volume up|down|mute` changes the volume (with
//! `wpctl`) and shows the launcher's volume bar. The launcher sees the
//! remote's volume keys itself only while it has the keyboard; to change
//! the volume while an app is in front, bind them in the compositor, e.g.
//! in labwc's `rc.xml`:
//!
//! ```xml
//! <keybind key="XF86AudioRaiseVolume">
//!   <action name="Execute" command="rusttv-launcher-linux --volume up" />
//! </keybind>
//! <keybind key="XF86AudioLowerVolume">
//!   <action name="Execute" command="rusttv-launcher-linux --volume down" />
//! </keybind>
//! <keybind key="XF86AudioMute">
//!   <action name="Execute" command="rusttv-launcher-linux --volume mute" />
//! </keybind>
//! ```

#[cfg(target_os = "linux")]
#[path = "linux/home.rs"]
mod home;

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    home::main()
}

/// The `linux` feature builds on other systems (for `--all-features`), but
/// this binary does nothing there.
#[cfg(not(target_os = "linux"))]
fn main() {}
