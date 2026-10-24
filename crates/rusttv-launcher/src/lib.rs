//! RustTV home screen (ARCHITECTURE.md § Launcher).
//!
//! [`Home`] holds the screen state: tile order, D-pad focus, scrolling, move
//! mode and launches. It has no UI or platform code; the caller feeds it
//! [`Input`]s and carries out the returned [`Effect`]s. A [`Platform`] lists
//! and launches apps and updates the launcher, and [`run`] shows the Slint
//! home screen.

#[cfg(target_os = "android")]
mod android;
mod history;
mod i18n;
mod model;
mod order_file;
mod platform;
mod review;
mod settings;
mod ui;

pub use model::{
    App, Effect, Home, Input, LAUNCH_TIMEOUT, LONG_PRESS, MOVE_IDLE_TIMEOUT, MenuItem, Order,
    Scroll, State, TileId,
};
#[cfg(any(test, feature = "test-util"))]
pub use platform::FakePlatform;
pub use platform::{AppEntry, Artwork, Bitmap, Platform, PlatformError, UpdateStatus};
#[cfg(any(test, feature = "test-util"))]
pub use ui::{DemoStep, run_demo};
pub use ui::{Lifecycle, RunOptions, run};

/// Starts the history thread for the desktop dev mode over `dir`, the
/// directory of the order file (HISTORY.md).
#[cfg(not(target_os = "android"))]
pub fn start_desktop_history(dir: std::path::PathBuf) {
    history::desktop::start(dir);
}
