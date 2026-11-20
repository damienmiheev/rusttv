//! A remote desktop portal backend for the TV: apps such as KDE Connect
//! (the phone as touchpad and keyboard) control the TV through
//! `xdg-desktop-portal`, in compositors that have no such portal of their
//! own (labwc). Only KDE Connect is allowed; see `src/linux/portal.rs`.
//!
//! The session bus starts it when `xdg-desktop-portal` first asks for
//! `org.freedesktop.impl.portal.desktop.rusttv`.

/// Logs info, warnings and errors to stderr (the session's journal).
#[cfg(target_os = "linux")]
struct Stderr;

#[cfg(target_os = "linux")]
impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    #[expect(clippy::print_stderr, reason = "stderr is the service's log")]
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) && record.target().starts_with("rusttv") {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    rusttv_launcher::run_portal()
}

/// The `linux` feature builds on other systems (for `--all-features`), but
/// this binary does nothing there.
#[cfg(not(target_os = "linux"))]
fn main() {}
