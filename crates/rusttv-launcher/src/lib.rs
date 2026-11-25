//! RustTV home screen (ARCHITECTURE.md § Launcher).
//!
//! [`Home`] holds the screen state: tile order, D-pad focus, scrolling, move
//! mode and launches. It has no UI or platform code; the caller feeds it
//! [`Input`]s and carries out the returned [`Effect`]s. A [`Platform`] lists
//! and launches apps and updates the launcher, and [`run`] shows the Slint
//! home screen.

#[cfg(target_os = "android")]
mod android;
mod device;
mod history;
mod i18n;
#[cfg(all(target_os = "linux", feature = "linux"))]
mod linux;
mod model;
mod order_file;
mod panels;
mod platform;
mod remote;
mod remote_page;
mod review;
mod settings;
#[cfg(feature = "skins")]
mod skin_runtime;
/// The panel template converter build.rs runs, compiled here for its tests.
#[cfg(test)]
#[path = "../codegen/convert.rs"]
mod template_convert;
#[cfg(test)]
mod template_convert_tests;
mod theme;
mod ui;

#[cfg(any(test, feature = "test-util"))]
pub use device::FakeDevice;
pub use device::{
    Control, ControlKind, DeviceSettings, QuickPanel, Tab, Value, Volume, VolumeChange,
};
#[cfg(any(test, feature = "test-util"))]
pub use i18n::is_right_to_left;
#[cfg(all(target_os = "linux", feature = "linux"))]
pub use linux::{
    AppEvent, AppEvents, Limits, LinuxDevice, LinuxPlatform, run_portal, start_gamepads,
    start_keyboard,
};
pub use model::{
    App, Effect, Home, Input, LAUNCH_TIMEOUT, LONG_PRESS, MOVE_IDLE_TIMEOUT, MenuItem, Order,
    QuickAction, Scroll, SizeStep, State, TileId,
};
#[cfg(any(test, feature = "test-util"))]
pub use platform::FakePlatform;
pub use platform::{AppEntry, Artwork, Bitmap, Platform, PlatformError, UpdateStatus};
#[cfg(any(test, feature = "test-util"))]
pub use ui::{DemoStep, run_demo};
pub use ui::{Lifecycle, RunOptions, run};

/// Starts the history thread for the desktop dev mode over `dir`, the
/// directory of the order file (HISTORY.md). Without `grant`, as on a TV
/// without `WRITE_SECURE_SETTINGS`, an approved settings change waits for
/// the computer (HISTORY.md § 6.3), which shows History's waiting approval.
#[cfg(not(target_os = "android"))]
pub fn start_desktop_history(dir: std::path::PathBuf, grant: bool) {
    history::desktop::start(dir, grant);
}
