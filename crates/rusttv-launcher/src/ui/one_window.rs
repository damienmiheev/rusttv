//! Home and a programmable skin share one window.
//!
//! Wayland can't hide a window, only destroy it, so swapping between two
//! windows (the skin's for Home, `HomeWindow` for Settings, folders and the
//! other built-in pages) destroyed one and created the other. The new window
//! appears only after a configure round trip and its first frame, and in
//! between the app behind the launcher (the last one opened) showed through.
//!
//! Instead the skin is created on `HomeWindow`'s window
//! ([`SkinInstance::load_into_window`](crate::skin_runtime::SkinInstance::load_into_window))
//! and the window is switched between the two item trees, as Slint's own live
//! preview does when it reloads into the same window. The skin stays its own
//! component, driven only through its `RustTV` global; only the surface is
//! shared. The window's event hook is `HomeWindow`'s (`pause_when_covered`),
//! which serves both.

use slint::ComponentHandle as _;
use slint::private_unstable_api::re_exports::{ItemTreeRc, VRc, WindowInner};

use super::HomeWindow;

/// The two item trees the window shows.
pub(super) struct Trees {
    home: ItemTreeRc,
    skin: ItemTreeRc,
}

impl Trees {
    /// The tree `window` shows now.
    pub(super) fn current(window: &HomeWindow) -> ItemTreeRc {
        WindowInner::from_pub(window.window()).component()
    }

    /// `home` is `HomeWindow`'s tree and `skin` the skin's, as
    /// [`Self::current`] returned them.
    pub(super) fn new(home: ItemTreeRc, skin: ItemTreeRc) -> Self {
        Self { home, skin }
    }

    /// `HomeWindow`'s tree.
    pub(super) fn home(&self) -> &ItemTreeRc {
        &self.home
    }

    /// Shows the skin. Switching trees drops the keyboard focus; the skin's
    /// first focusable element (its key `FocusScope`) gets it back.
    pub(super) fn show_skin(&self, window: &HomeWindow) {
        show(window);
        let inner = WindowInner::from_pub(window.window());
        if !VRc::ptr_eq(&inner.component(), &self.skin) {
            inner.set_component(&self.skin);
            inner.focus_next_item();
        }
    }

    /// Shows `HomeWindow`'s pages, with the keyboard on its key handler.
    pub(super) fn show_home(&self, window: &HomeWindow) {
        show_home_tree(window, &self.home);
    }
}

/// Shows `home` (`HomeWindow`'s tree) in `window`, with the keyboard on its
/// key handler.
pub(super) fn show_home_tree(window: &HomeWindow, home: &ItemTreeRc) {
    show(window);
    let inner = WindowInner::from_pub(window.window());
    if !VRc::ptr_eq(&inner.component(), home) {
        inner.set_component(home);
        window.invoke_focus_keys();
    }
}

/// Maps the window if it isn't yet: it stays up from then on.
fn show(window: &HomeWindow) {
    window.window().set_fullscreen(true);
    let _ = window.show();
}
