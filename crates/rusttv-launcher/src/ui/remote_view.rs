//! RustTV Remote in Settings (REMOTE.md § 2): the pairing screen (QR code,
//! then "Allow?" with the match code and OK on the remote), the paired
//! phones, and removing one. Phones can never do any of this themselves:
//! ui.rs drops their input while the pairing screen is open.

use std::time::Duration;

use slint::{Image, Rgba8Pixel, SharedPixelBuffer, Timer};

use super::{Controller, HomeWindow, settings_view};
use crate::model::Input;
use crate::remote;

/// How long "paired" shows before the screen goes back to Settings.
const PAIRED_SHOWN: Duration = Duration::from_secs(2);

/// Pixels per QR module; Slint scales the image to the screen without
/// smoothing.
const QR_SCALE: usize = 8;

/// The pairing screen's state.
pub(super) struct Pairing {
    phase: Phase,
    qr: Image,
    address: String,
}

enum Phase {
    /// The QR code is up.
    Waiting,
    /// A phone that read it asks to pair.
    Asking {
        request: u64,
        name: String,
        code: u8,
    },
    Paired {
        name: String,
    },
    /// `Strings.pairing-failure` kinds: 1 port taken, 2 no address, 3 no
    /// code, 4 three wrong tries.
    Failed(i32),
}

impl Controller {
    /// OK on "Pair a phone": starts the listener if needed, opens pairing
    /// and shows the QR code.
    pub(super) fn open_pairing(&mut self) {
        if self.remote.is_none() {
            self.start_remote_now(true);
        }
        let failed = |kind: i32| Pairing {
            phase: Phase::Failed(kind),
            qr: Image::default(),
            address: String::new(),
        };
        let pairing = match (&self.remote, remote::lan_address()) {
            (None, _) => failed(1),
            (Some(_), None) => failed(2),
            (Some(link), Some(address)) => {
                let link_text = link.open_pairing(&address.to_string());
                match remote::qr_pixels(&link_text, QR_SCALE) {
                    Some((side, pixels)) => Pairing {
                        phase: Phase::Waiting,
                        qr: Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                            &pixels, side, side,
                        )),
                        address: format!("http://{address}:{}", link.remote().address().port()),
                    },
                    None => failed(3),
                }
            }
        };
        // The code carries a pairing secret: keep it out of screen captures.
        self.platform.set_secure(true);
        self.pairing = Some(pairing);
        self.render();
    }

    /// A key while the pairing screen is open: OK allows a phone that asks,
    /// Back closes (refusing it).
    pub(super) fn pairing_input(&mut self, input: Input) {
        let Some(pairing) = &self.pairing else {
            return;
        };
        match (&pairing.phase, input) {
            (Phase::Asking { request, name, .. }, Input::OkReleased) => {
                let (request, name) = (*request, name.clone());
                if let Some(link) = &self.remote {
                    link.remote().answer_pairing(request, true);
                }
                if let Some(pairing) = &mut self.pairing {
                    pairing.phase = Phase::Paired { name };
                }
                // Back to Settings by itself, where the phone is listed.
                let me = self.me.clone();
                Timer::single_shot(PAIRED_SHOWN, move || {
                    if let Some(controller) = me.upgrade() {
                        let mut controller = controller.borrow_mut();
                        if controller
                            .pairing
                            .as_ref()
                            .is_some_and(|pairing| matches!(pairing.phase, Phase::Paired { .. }))
                        {
                            controller.close_pairing();
                        }
                    }
                });
            }
            (_, Input::Back) => self.close_pairing(),
            _ => return,
        }
        self.render();
    }

    /// Leaves the pairing screen; a phone still asking is refused.
    pub(super) fn close_pairing(&mut self) {
        let Some(pairing) = self.pairing.take() else {
            return;
        };
        self.platform.set_secure(false);
        if let Some(link) = &self.remote {
            if let Phase::Asking { request, .. } = pairing.phase {
                link.remote().answer_pairing(request, false);
            }
            link.remote().close_pairing();
        }
        self.phones_changed();
        self.render();
    }

    /// A phone read the code and proved it (REMOTE.md § 2).
    pub(super) fn pair_request(&mut self, request: u64, name: String, code: u8) {
        match &mut self.pairing {
            Some(pairing) if matches!(pairing.phase, Phase::Waiting) => {
                pairing.phase = Phase::Asking {
                    request,
                    name,
                    code,
                };
                self.render();
            }
            // The desktop dev mode allows every phone; on a TV a request
            // with no pairing screen open is refused.
            _ => {
                let allow = self.options.remote_pairing;
                log::info!("RustTV Remote: {name} asks to pair (code {code:02}); allowed: {allow}");
                if let Some(link) = &self.remote {
                    link.remote().answer_pairing(request, allow);
                }
            }
        }
    }

    /// Too many wrong pairing tries: the code no longer works.
    pub(super) fn pairing_closed(&mut self) {
        if let Some(pairing) = &mut self.pairing {
            pairing.phase = Phase::Failed(4);
            self.render();
        }
    }

    /// Unpairs a phone (OK twice on its row): it can't connect any more.
    pub(super) fn remove_phone(&mut self, id: rusttv_remote::PhoneId) {
        let Some(link) = &self.remote else {
            return;
        };
        if let Some(phone) = link.remote().revoke(&id) {
            link.save_phones();
            self.settings_status = (settings_view::REMOVED_PHONE, phone.name);
        }
        self.phones_changed();
    }

    /// Settings lists the phones again; with none left (and no pairing
    /// screen), the listener stops.
    pub(super) fn phones_changed(&mut self) {
        let rows = self
            .remote
            .as_ref()
            .map(remote::Link::phone_rows)
            .unwrap_or_default();
        if rows.is_empty() && self.pairing.is_none() && !self.options.remote_pairing {
            // Dropping the link stops the listener and its threads.
            self.remote = None;
        }
        if let Some(page) = &mut self.settings {
            page.set_phones(rows);
        }
    }

    /// Fills the pairing screen's properties.
    pub(super) fn render_pairing(&self, window: &HomeWindow) {
        let Some(pairing) = &self.pairing else {
            return;
        };
        let (phase, name, code, failure) = match &pairing.phase {
            Phase::Waiting => (0, "", String::new(), 0),
            Phase::Asking { name, code, .. } => (1, name.as_str(), format!("{code:02}"), 0),
            Phase::Paired { name } => (2, name.as_str(), String::new(), 0),
            Phase::Failed(kind) => (3, "", String::new(), *kind),
        };
        window.set_pairing_phase(phase);
        window.set_pairing_qr(pairing.qr.clone());
        window.set_pairing_address(pairing.address.as_str().into());
        window.set_pairing_name(name.into());
        window.set_pairing_code(code.into());
        window.set_pairing_failure(failure);
    }
}
