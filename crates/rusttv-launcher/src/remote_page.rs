//! The RustTV Remote page (REMOTE.md § 2), opened from its own Home tile:
//! "Pair a phone", "Pointer in other apps" (its accessibility service, § 5),
//! then the paired phones, each removed with OK twice. No UI or platform
//! code; ui.rs draws it and carries out the returned [`Action`]s.

use crate::model::Input;
use crate::platform::PointerState;

/// A paired phone, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhoneRow {
    pub id: rusttv_remote::PhoneId,
    pub name: String,
    /// Unix seconds of its last connection.
    pub last_seen: Option<u64>,
}

/// Something ui.rs must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Leave the page.
    Close,
    /// Show the pairing screen (QR code, then "Allow?").
    PairPhone,
    /// Unpair this phone (OK twice on its row).
    RemovePhone(rusttv_remote::PhoneId),
    /// Open Android's accessibility settings, where the pointer's service is
    /// switched on or off.
    PointerSettings,
}

/// Row 0 is "Pair a phone", row 1 "Pointer in other apps", then the phones.
const PHONES_FROM: usize = 2;

/// The page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RemotePage {
    phones: Vec<PhoneRow>,
    pointer: PointerState,
    row: usize,
    /// The phone row whose first OK asked "remove?".
    removing: Option<usize>,
}

impl RemotePage {
    /// The paired phones changed (paired, removed, connected).
    pub fn set_phones(&mut self, phones: Vec<PhoneRow>) {
        self.phones = phones;
        self.removing = None;
        self.row = self.row.min(self.last_row());
    }

    /// Where the pointer's service stands now.
    pub fn set_pointer(&mut self, pointer: PointerState) {
        self.pointer = pointer;
    }

    /// Where the pointer's service stands.
    pub fn pointer(&self) -> PointerState {
        self.pointer
    }

    /// The paired phones.
    pub fn phones(&self) -> &[PhoneRow] {
        &self.phones
    }

    /// The focused row.
    pub fn row(&self) -> usize {
        self.row
    }

    fn last_row(&self) -> usize {
        PHONES_FROM + self.phones.len() - 1
    }

    /// The phone row waiting for a second OK to be removed.
    pub fn removing(&self) -> Option<usize> {
        self.removing
    }

    /// Applies one key; returns what ui.rs must do.
    pub fn handle(&mut self, input: Input) -> Option<Action> {
        // Only keys that act count: OK's press, repeats and ticks come
        // between two OKs and must not cancel the question.
        if !matches!(
            input,
            Input::Up | Input::Down | Input::Back | Input::OkReleased
        ) {
            return None;
        }
        // A second OK must come right after the first, on the same row.
        let removing = self.removing.take();
        match input {
            Input::Up => self.row = self.row.saturating_sub(1),
            Input::Down => self.row = (self.row + 1).min(self.last_row()),
            Input::Back => return Some(Action::Close),
            Input::OkReleased if self.row == 0 => return Some(Action::PairPhone),
            Input::OkReleased if self.row == 1 => {
                return (self.pointer != PointerState::Unavailable)
                    .then_some(Action::PointerSettings);
            }
            Input::OkReleased => {
                let phone = self.phones.get(self.row - PHONES_FROM)?;
                if removing == Some(self.row) {
                    return Some(Action::RemovePhone(phone.id));
                }
                self.removing = Some(self.row);
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(byte: u8, name: &str) -> PhoneRow {
        PhoneRow {
            id: rusttv_remote::PhoneId([byte; 16]),
            name: name.to_owned(),
            last_seen: None,
        }
    }

    #[test]
    fn pairs_and_removes_with_a_second_ok() {
        let mut page = RemotePage::default();
        page.set_phones(vec![phone(1, "iPhone"), phone(2, "Pixel")]);
        assert_eq!(page.row(), 0);
        assert_eq!(page.handle(Input::OkReleased), Some(Action::PairPhone));
        page.handle(Input::Down);
        page.handle(Input::Down);
        assert_eq!(page.handle(Input::OkReleased), None);
        assert_eq!(page.removing(), Some(2));
        // The second OK's press comes first, as from a real remote.
        assert_eq!(page.handle(Input::OkPressed), None);
        assert_eq!(page.removing(), Some(2));
        assert_eq!(
            page.handle(Input::OkReleased),
            Some(Action::RemovePhone(rusttv_remote::PhoneId([1; 16])))
        );
        // Moving away cancels the question.
        page.handle(Input::OkReleased);
        page.handle(Input::Down);
        assert_eq!(page.removing(), None);
        assert_eq!(page.handle(Input::OkReleased), None);
        // Fewer phones: focus stays on a row that exists.
        page.set_phones(vec![phone(1, "iPhone")]);
        assert_eq!(page.row(), 2);
        assert_eq!(page.removing(), None);
        assert_eq!(page.handle(Input::Back), Some(Action::Close));
    }

    #[test]
    fn the_pointer_row_opens_accessibility_settings_where_there_are_some() {
        let mut page = RemotePage::default();
        page.handle(Input::Down);
        assert_eq!(page.row(), 1);
        // The desktop has no pointer: the row does nothing.
        assert_eq!(page.handle(Input::OkReleased), None);
        for state in [PointerState::Off, PointerState::Ready] {
            page.set_pointer(state);
            assert_eq!(
                page.handle(Input::OkReleased),
                Some(Action::PointerSettings)
            );
            assert_eq!(page.removing(), None);
        }
    }
}
