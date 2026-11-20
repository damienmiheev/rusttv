//! The RustTV Remote panel (PANELS.md § 10 step 3): its tile's values, and
//! its page (REMOTE.md § 2), opened from its Home tile: the
//! pairing screen (QR code, then "Allow?" with the match code and OK on the
//! remote), the pointer's row, the paired phones, and removing one. Phones can never do any of
//! this themselves: ui.rs passes them only Back on this page and nothing
//! while the pairing screen is open.

use std::time::Duration;

use slint::{ComponentHandle, Image, Model, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

use super::{Controller, HomeWindow, RemotePanel, SettingsRow, Strings, settings_view};
use crate::model::Input;
use crate::panels;
use crate::platform::PointerState;
use crate::remote;
use crate::remote_page::{Action, RemotePage};

/// How long "paired" shows before the screen goes back to the page.
const PAIRED_SHOWN: Duration = Duration::from_secs(2);
/// How long the check code shows while a phone sets up trust.
const TRUST_SHOWN: Duration = Duration::from_secs(180);

/// Pixels per QR module; Slint scales the image to the screen without
/// smoothing.
const QR_SCALE: usize = 8;

/// Phone rows the page shows at once; focus scrolls past them.
const PAGE_ROWS: usize = 6;

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
    /// A paired phone sets up trust in the TV's certificate: the code to
    /// compare on the phone before installing (REMOTE.md § 2).
    Trust {
        name: String,
        certificate: String,
        code: String,
    },
}

impl Controller {
    /// OK on the RustTV Remote tile: the page, with the paired phones.
    pub(super) fn open_remote_page(&mut self) {
        let mut page = RemotePage::default();
        page.set_pointer(self.platform.pointer_state());
        self.remote_page = Some(page);
        self.settings_status = (settings_view::NO_STATUS, String::new());
        self.phones_changed();
    }

    /// A key while the RustTV Remote page (or its pairing screen) is open.
    pub(super) fn remote_page_input(&mut self, input: Input) {
        if self.pairing.is_some() {
            self.pairing_input(input);
            return;
        }
        let Some(page) = &mut self.remote_page else {
            return;
        };
        if matches!(input, Input::Up | Input::Down | Input::OkReleased) {
            self.settings_status = (settings_view::NO_STATUS, String::new());
        }
        match page.handle(input) {
            Some(Action::Close) => {
                self.remote_page = None;
                self.home.close_remote();
                // With no phone left, the listener stops.
                self.phones_changed();
            }
            Some(Action::PairPhone) => self.open_pairing(),
            Some(Action::RemovePhone(id)) => self.remove_phone(id),
            Some(Action::PointerSettings) => self.platform.open_pointer_settings(),
            None => {}
        }
    }

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
        // The listener's secret is live whether or not the code could be
        // drawn (failure 3), so the tile says "Pairing" until it closes.
        if matches!(pairing.phase, Phase::Waiting | Phase::Failed(3)) {
            self.pairing_opened();
        }
        // The code carries a pairing secret: keep it out of screen captures.
        self.platform.set_secure(true);
        self.pairing = Some(pairing);
        self.show_remote_panel();
        self.render();
    }

    /// A paired phone is setting up trust: shows the TV's check code, over
    /// whatever RustTV shows, until Back or [`TRUST_SHOWN`]. Not while
    /// another app is in front, pairing runs or the review screen is open.
    pub(super) fn open_trust(&mut self, phone: rusttv_remote::PhoneId) {
        if self.away || self.pairing.is_some() || self.review.is_some() {
            return;
        }
        let Some(link) = &self.remote else {
            return;
        };
        let Some((certificate, code)) = link.certificate() else {
            return;
        };
        let name = link
            .phone_rows()
            .into_iter()
            .find(|row| row.id == phone)
            .map(|row| row.name)
            .unwrap_or_default();
        self.pairing = Some(Pairing {
            phase: Phase::Trust {
                name,
                certificate,
                code,
            },
            qr: Image::default(),
            address: String::new(),
        });
        let me = self.me.clone();
        Timer::single_shot(TRUST_SHOWN, move || {
            if let Some(controller) = me.upgrade() {
                let mut controller = controller.borrow_mut();
                if controller
                    .pairing
                    .as_ref()
                    .is_some_and(|pairing| matches!(pairing.phase, Phase::Trust { .. }))
                {
                    controller.close_pairing();
                }
            }
        });
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
                // Back to the page by itself, where the phone is listed.
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
        self.pairing_timer.stop();
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
        self.pairing_timer.stop();
        self.show_remote_panel();
        if let Some(pairing) = &mut self.pairing {
            pairing.phase = Phase::Failed(4);
            self.render();
        }
    }

    /// Pairing opened (the listener has a fresh secret for
    /// [`rusttv_remote::PAIRING_TIME`]): the tile says "Pairing" until it
    /// closes, a phone pairs, or that time is up. It follows the secret, not
    /// the pairing screen (which stays until Back), and its timer runs on
    /// Home's clock, so it may end a moment after the listener's.
    pub(super) fn pairing_opened(&mut self) {
        let me = self.me.clone();
        self.pairing_timer.start(
            TimerMode::SingleShot,
            rusttv_remote::PAIRING_TIME,
            move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow().show_remote_panel();
                }
            },
        );
    }

    /// Sets the RustTV Remote tile's values (src/panels.rs `remote_view`):
    /// window-global properties, so a change redraws the tile without
    /// touching the tile list. Called from the remote's events (pairing,
    /// a phone paired or removed), never polled.
    pub(super) fn show_remote_panel(&self) {
        let phones = self
            .remote
            .as_ref()
            .map_or(0, |link| link.remote().phones().len());
        let pairing = self.pairing_timer.running();
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if let Some(tile) = panels::remote_view(phones, pairing).tile {
            let panel = window.global::<RemotePanel>();
            panel.set_phones(tile.props.get("phones").into());
            panel.set_status(tile.props.get("status").into());
        }
    }

    /// Unpairs a phone (OK twice on its row): it can't connect any more.
    pub(super) fn remove_phone(&mut self, id: rusttv_remote::PhoneId) {
        let Some(link) = &self.remote else {
            return;
        };
        if let Some(phone) = link.remote().revoke(&id) {
            link.save_phones();
            if let Some(history) = crate::history::History::get() {
                history.launcher_edited();
            }
            self.settings_status = (settings_view::REMOVED_PHONE, phone.name);
        }
        self.phones_changed();
    }

    /// The page lists the phones again; with none left (and neither the
    /// page nor the pairing screen open), the listener stops.
    pub(super) fn phones_changed(&mut self) {
        let rows = self
            .remote
            .as_ref()
            .map(remote::Link::phone_rows)
            .unwrap_or_default();
        if rows.is_empty()
            && self.pairing.is_none()
            && self.remote_page.is_none()
            && !self.options.remote_pairing
        {
            // Dropping the link stops the listener and its threads.
            self.remote = None;
        }
        if let Some(page) = &mut self.remote_page {
            page.set_phones(rows);
        }
        self.show_remote_panel();
    }

    /// Fills the RustTV Remote page's rows.
    pub(super) fn render_remote_page(&self, window: &HomeWindow) {
        let Some(page) = &self.remote_page else {
            return;
        };
        let strings = window.global::<Strings>();
        let pointer = match page.pointer() {
            PointerState::Unavailable => 0,
            PointerState::Off => 1,
            PointerState::Ready => 2,
        };
        let mut rows = vec![
            SettingsRow {
                title: strings.invoke_pair_phone(),
                detail: strings.invoke_pair_phone_detail(),
                current: false,
                dim: false,
                ..SettingsRow::default()
            },
            SettingsRow {
                title: strings.invoke_pointer_row(),
                detail: strings.invoke_pointer_detail(pointer),
                current: pointer == 2,
                dim: pointer == 0,
                ..SettingsRow::default()
            },
        ];
        rows.extend(
            page.phones()
                .iter()
                .enumerate()
                .map(|(i, phone)| SettingsRow {
                    title: phone.name.as_str().into(),
                    detail: strings.invoke_phone_detail(page.removing() == Some(i + 2)),
                    current: false,
                    dim: false,
                    ..SettingsRow::default()
                }),
        );
        let row = page.row();
        window.set_remote_row(i32::try_from(row).unwrap_or(i32::MAX));
        window.set_remote_first_row(
            i32::try_from(row.saturating_sub(PAGE_ROWS - 1)).unwrap_or(i32::MAX),
        );
        if !self.remote_rows.iter().eq(rows.iter().cloned()) {
            self.remote_rows.set_vec(rows);
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
            Phase::Trust { name, code, .. } => (4, name.as_str(), code.clone(), 0),
        };
        window.set_pairing_phase(phase);
        window.set_pairing_qr(pairing.qr.clone());
        let address = match &pairing.phase {
            Phase::Trust { certificate, .. } => certificate.as_str(),
            _ => pairing.address.as_str(),
        };
        window.set_pairing_address(address.into());
        window.set_pairing_name(name.into());
        window.set_pairing_code(code.into());
        window.set_pairing_failure(failure);
    }
}
