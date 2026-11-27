//! Home's own faces of the panels (PANELS.md § 2, § 9 Q2, Q4, Q6): the
//! status corner (the clock and the glyphs of panels that need attention),
//! banners (one at a time, over Home only) and the idle screen (a dim clock
//! and the panels' lines after [`AMBIENT_AFTER`] on Home).
//!
//! No timer of its own beyond the clock's minute (`Faces::clock_timer`,
//! which also moves the idle screen) and the banner's few seconds: the idle
//! screen starts from the sway's idle timer (ui.rs `restart_idle`), and the
//! glyphs and lines change with the panels' own events (the updater's
//! status, pairing, History's summary), never per frame.

use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::{
    AboutAmbient, AboutBanner, AboutCorner, AmbientLine, Controller, HistoryAmbient, HistoryBanner,
    HistoryCorner, HomeWindow, RemoteAmbient, RemoteBanner, RemoteCorner,
};
use crate::model::State;
use crate::panels::{self, PanelBanner, Props};

/// How long Home rests before it dims to the idle screen (owner's decision
/// delegated, 2026-11-21): long enough never to start while someone reads
/// Home, well before most TVs' own screensaver (which then takes over: the
/// launcher pauses and the idle screen goes). Settings says "5 minutes"
/// (home.slint `ambient-detail`; a test keeps the two together).
pub(super) const AMBIENT_AFTER: Duration = Duration::from_secs(5 * 60);

/// A banner that waited longer than this (Home wasn't plain: a page, a
/// folder, a menu was open) is dropped: by then it isn't news.
const BANNER_FRESH: Duration = Duration::from_secs(30);

/// How long a banner stays.
const BANNER_TIME: Duration = Duration::from_secs(4);

/// The pause between two banners: the first fades out (home.slint) first.
const BANNER_GAP: Duration = Duration::from_millis(400);

/// Banners kept waiting at most; an older one goes first (queued, never
/// stacked on screen).
const BANNERS_KEPT: usize = 4;

/// A status-corner glyph shows at most this many panels (Q2).
const GLYPHS: usize = 2;

/// Which panel a banner is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Panel {
    About,
    Remote,
    History,
}

/// A banner's words and picture, in the language at the time it came, and
/// when it came (the controller's clock).
#[derive(Debug, Clone)]
struct Banner {
    title: SharedString,
    detail: SharedString,
    badge: Image,
    since: Duration,
}

/// The faces' state in the controller.
pub(super) struct Faces {
    /// Fires at each minute's start while Home is on screen: the clock.
    clock_timer: Timer,
    /// Banners waiting; the first is on screen while `banner_up`.
    banners: VecDeque<Banner>,
    banner_up: bool,
    /// Ends the banner on screen, then waits [`BANNER_GAP`].
    banner_timer: Timer,
    /// The idle screen is on.
    pub ambient: bool,
    /// The idle screen's setting (Settings → Idle screen; on by default).
    pub ambient_on: bool,
    /// The key that woke the idle screen: its repeats and release are
    /// swallowed too, so it does nothing on Home.
    pub ambient_key: Option<SharedString>,
    /// The idle screen's place, one step a minute (home.slint `drift`).
    drift: i32,
    /// Phones whose connection made a banner since the launcher started:
    /// one banner per phone, not one per reconnection.
    greeted: HashSet<rusttv_remote::PhoneId>,
    glyphs: Rc<VecModel<Image>>,
    lines: Rc<VecModel<AmbientLine>>,
}

impl Faces {
    /// The faces' models in `window`; the idle screen on unless `ambient_off`.
    pub(super) fn new(window: &HomeWindow, ambient_off: bool) -> Self {
        let glyphs = Rc::new(VecModel::default());
        window.set_corner_glyphs(ModelRc::from(glyphs.clone()));
        let lines = Rc::new(VecModel::default());
        window.set_ambient_lines(ModelRc::from(lines.clone()));
        Self {
            clock_timer: Timer::default(),
            banners: VecDeque::new(),
            banner_up: false,
            banner_timer: Timer::default(),
            ambient: false,
            ambient_on: !ambient_off,
            ambient_key: None,
            drift: 0,
            greeted: HashSet::new(),
            glyphs,
            lines,
        }
    }
}

/// "14:05", or "2:05" on a 12-hour clock (no AM/PM, as on a phone's status
/// bar), for `local_unix` (seconds, local time).
pub(super) fn clock_text(local_unix: i64, twenty_four: bool) -> String {
    let seconds = local_unix.rem_euclid(86_400);
    let (hours, minutes) = (seconds / 3600, seconds % 3600 / 60);
    if twenty_four {
        format!("{hours:02}:{minutes:02}")
    } else {
        let hours = match hours % 12 {
            0 => 12,
            hours => hours,
        };
        format!("{hours}:{minutes:02}")
    }
}

/// "GOOD MORNING" / "GOOD AFTERNOON" / "GOOD EVENING" for the greeting header skin.
pub(super) fn greeting_text(local_unix: i64) -> &'static str {
    let hour = local_unix.rem_euclid(86_400) / 3600;
    match hour {
        5..=11 => "GOOD MORNING",
        12..=17 => "GOOD AFTERNOON",
        _ => "GOOD EVENING",
    }
}

/// "SATURDAY  17 OCTOBER" for the greeting header skin (Howard Hinnant's
/// civil-from-days algorithm, zero extra dependencies).
pub(super) fn date_text(local_unix: i64) -> String {
    const DAYS: [&str; 7] = [
        "THURSDAY",
        "FRIDAY",
        "SATURDAY",
        "SUNDAY",
        "MONDAY",
        "TUESDAY",
        "WEDNESDAY",
    ];
    const MONTHS: [&str; 12] = [
        "JANUARY",
        "FEBRUARY",
        "MARCH",
        "APRIL",
        "MAY",
        "JUNE",
        "JULY",
        "AUGUST",
        "SEPTEMBER",
        "OCTOBER",
        "NOVEMBER",
        "DECEMBER",
    ];
    let z = local_unix.div_euclid(86_400);
    let weekday = DAYS[z.rem_euclid(7) as usize];
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    format!("{weekday}  {day} {}", MONTHS[month - 1])
}

/// A prop's value for a Slint call.
fn arg(props: &Props, name: &str) -> SharedString {
    props.get(name).into()
}

impl Controller {
    /// Sets the clock now and again at the next minute's start; the idle
    /// screen moves one step with it, and its History line gets its age.
    /// History's tile, row and lines get their age too (a copy of the
    /// history thread's summary, no history I/O), so a change the computer
    /// applied makes its banner within the minute. Runs while Home is on
    /// screen ([`Controller::faces_away`] stops it).
    pub(super) fn tick_clock(&mut self) {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now = i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX);
        let local = now.saturating_add(i64::from(self.platform.utc_offset_secs()));
        let twenty_four = self.platform.uses_24_hours().unwrap_or_else(|| {
            crate::i18n::uses_24_hours(self.options.language.as_deref().unwrap_or_default())
        });
        if let Some(window) = self.window.upgrade() {
            let clk = clock_text(local, twenty_four);
            let greet = greeting_text(local);
            let date = date_text(local);
            window.set_clock_text(clk.as_str().into());
            window.set_greeting_text(greet.into());
            window.set_date_text(date.as_str().into());
            #[cfg(feature = "skins")]
            if let Some(skin) = &self.active_skin {
                skin.set_clock(&clk, greet, &date);
            }
            if self.faces.ambient {
                self.faces.drift = (self.faces.drift + 1) % 25;
                window.set_ambient_drift(self.faces.drift);
            }
        }
        self.show_history_panel();
        let into_minute = u64::try_from(since_epoch.as_millis() % 60_000).unwrap_or_default();
        // A little after the minute starts, so the text is never a minute
        // behind.
        let wait = Duration::from_millis(60_000 - into_minute + 50);
        let me = self.me.clone();
        self.faces
            .clock_timer
            .start(TimerMode::SingleShot, wait, move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow_mut().tick_clock();
                }
            });
    }

    /// The launcher left the screen: the clock stops, the idle screen and
    /// banners go (banners that waited are dropped: they were about then).
    pub(super) fn faces_away(&mut self) {
        self.faces.clock_timer.stop();
        self.leave_ambient();
        // A release lost while away must not swallow the next press.
        self.faces.ambient_key = None;
        self.faces.banners.clear();
        self.faces.banner_timer.stop();
        if std::mem::take(&mut self.faces.banner_up)
            && let Some(window) = self.window.upgrade()
        {
            window.set_banner_shown(false);
        }
    }

    /// The panels' glyphs in the status corner (at most [`GLYPHS`], in
    /// About, RustTV Remote, History order) and their lines on the idle
    /// screen, from their views and templates. Sets the models only when
    /// they change.
    pub(super) fn show_faces(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let about = panels::about_view(env!("CARGO_PKG_VERSION"), &self.update);
        let remote = self.remote_panel_view();
        let history = panels::history_view(self.history.0.as_ref(), self.history.1);
        let mut glyphs = Vec::new();
        let mut lines = Vec::new();
        if let Some(corner) = about.corner {
            let face = window.global::<AboutCorner>();
            if face.invoke_show(arg(&corner.props, "status")) {
                glyphs.push(face.get_badge());
            }
        }
        if let Some(corner) = remote.corner {
            let face = window.global::<RemoteCorner>();
            if face.invoke_show(arg(&corner.props, "status")) {
                glyphs.push(face.get_badge());
            }
        }
        if let Some(corner) = history.corner {
            let face = window.global::<HistoryCorner>();
            if face.invoke_show(arg(&corner.props, "status")) {
                glyphs.push(face.get_badge());
            }
        }
        if let Some(ambient) = about.ambient {
            let face = window.global::<AboutAmbient>();
            let status = arg(&ambient.props, "status");
            if face.invoke_show(status.clone()) {
                lines.push(AmbientLine {
                    badge: face.get_badge(),
                    text: face.invoke_text(status),
                });
            }
        }
        if let Some(ambient) = remote.ambient {
            let face = window.global::<RemoteAmbient>();
            let status = arg(&ambient.props, "status");
            if face.invoke_show(status.clone()) {
                lines.push(AmbientLine {
                    badge: face.get_badge(),
                    text: face.invoke_text(status),
                });
            }
        }
        if let Some(ambient) = history.ambient {
            let face = window.global::<HistoryAmbient>();
            let (status, count) = (arg(&ambient.props, "status"), arg(&ambient.props, "count"));
            if face.invoke_show(status.clone(), count.clone()) {
                lines.push(AmbientLine {
                    badge: face.get_badge(),
                    text: face.invoke_text(status, count),
                });
            }
        }
        glyphs.truncate(GLYPHS);
        if !self.faces.glyphs.iter().eq(glyphs.iter().cloned()) {
            self.faces.glyphs.set_vec(glyphs);
        }
        if !self.faces.lines.iter().eq(lines.iter().cloned()) {
            self.faces.lines.set_vec(lines);
        }
    }

    /// Queues a panel's banner (its `banner` template's words, now); it
    /// shows when Home can ([`Controller::show_banners`]). Dropped while
    /// the launcher is away: it was about then.
    pub(super) fn queue_banner(&mut self, panel: Panel, banner: &PanelBanner) {
        if self.away {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let props = &banner.props;
        let since = (self.clock)();
        let banner = match panel {
            Panel::About => {
                let face = window.global::<AboutBanner>();
                let (event, version) = (arg(props, "event"), arg(props, "version"));
                Banner {
                    title: face.invoke_title(event.clone(), version.clone()),
                    detail: face.invoke_detail(event, version),
                    badge: face.get_badge(),
                    since,
                }
            }
            Panel::Remote => {
                let face = window.global::<RemoteBanner>();
                let phone = arg(props, "phone");
                Banner {
                    title: face.invoke_title(phone.clone()),
                    detail: face.invoke_detail(phone),
                    badge: face.get_badge(),
                    since,
                }
            }
            Panel::History => {
                let face = window.global::<HistoryBanner>();
                let count = arg(props, "count");
                Banner {
                    title: face.invoke_title(count.clone()),
                    detail: face.invoke_detail(count),
                    badge: face.get_badge(),
                    since,
                }
            }
        };
        // The one on screen stays first; an older waiting one makes room.
        if self.faces.banners.len() >= BANNERS_KEPT {
            let first_waiting = usize::from(self.faces.banner_up);
            self.faces.banners.remove(first_waiting);
        }
        self.faces.banners.push_back(banner);
        self.show_banners();
    }

    /// A phone connected: its banner, once per phone since the launcher
    /// started.
    pub(super) fn phone_connected(&mut self, phone: rusttv_remote::PhoneId) {
        let name = self
            .remote
            .as_ref()
            .and_then(|link| link.remote().phones().get(&phone).map(|p| p.name.clone()));
        if let Some(name) = name
            && self.faces.greeted.insert(phone)
        {
            self.queue_banner(Panel::Remote, &panels::remote_banner(&name));
        }
    }

    /// Whether Home can show a banner: plain Home in front, nothing open
    /// over it (a page, a folder, a menu, the top-right menu, a prompt, the
    /// review or pairing screen), on its first row (with the hero and the
    /// status corner, never over the tiles) and not the idle screen.
    fn banner_allowed(&self) -> bool {
        self.home_alone()
            && !self.faces.ambient
            && matches!(self.home.scroll(), crate::model::Scroll::Shelf)
    }

    /// Plain Home in front: browsing the tiles, with nothing over them.
    fn home_alone(&self) -> bool {
        !self.away
            && !self.overlay
            && self.review.is_none()
            && self.pairing.is_none()
            && self.prompt.is_none()
            && self.renaming.is_none()
            && self.home.state() == State::Browse
    }

    /// Shows the next banner when Home can, or takes the one on screen
    /// away at once when Home can't any more (it shows again, whole, when
    /// Home can). Called after every change Home renders.
    pub(super) fn show_banners(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let allowed = self.banner_allowed();
        if self.faces.banner_up {
            if !allowed {
                self.faces.banner_up = false;
                self.faces.banner_timer.stop();
                window.set_banner_shown(false);
            }
            return;
        }
        if !allowed || self.faces.banner_timer.running() {
            return;
        }
        // Banners that waited too long are old news.
        let now = (self.clock)();
        while self
            .faces
            .banners
            .front()
            .is_some_and(|banner| now.saturating_sub(banner.since) > BANNER_FRESH)
        {
            self.faces.banners.pop_front();
        }
        let Some(banner) = self.faces.banners.front() else {
            return;
        };
        window.set_banner_title(banner.title.clone());
        window.set_banner_detail(banner.detail.clone());
        window.set_banner_badge(banner.badge.clone());
        window.set_banner_shown(true);
        self.faces.banner_up = true;
        let me = self.me.clone();
        self.faces
            .banner_timer
            .start(TimerMode::SingleShot, BANNER_TIME, move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow_mut().banner_done();
                }
            });
    }

    /// The banner's time is up: it fades out, and the next waits
    /// [`BANNER_GAP`].
    fn banner_done(&mut self) {
        self.faces.banners.pop_front();
        self.faces.banner_up = false;
        if let Some(window) = self.window.upgrade() {
            window.set_banner_shown(false);
        }
        let me = self.me.clone();
        self.faces
            .banner_timer
            .start(TimerMode::SingleShot, BANNER_GAP, move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow_mut().show_banners();
                }
            });
    }

    /// Starts the idle screen's wait (`after`) on the sway's idle timer.
    pub(super) fn arm_ambient(&self, after: Duration) {
        let me = self.me.clone();
        self.idle_timer
            .start(TimerMode::SingleShot, after, move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow_mut().enter_ambient();
                }
            });
    }

    /// Dims Home to the idle screen if it is on and Home is alone in front
    /// (never over another app, a page or the review screen). When Home
    /// isn't alone (a page stayed open), it waits another
    /// [`AMBIENT_AFTER`]: a page or screen that closes without a key (a
    /// phone, a time limit) still gets the idle screen later.
    pub(super) fn enter_ambient(&mut self) {
        if !self.faces.ambient_on || self.faces.ambient || self.away {
            return;
        }
        if !self.home_alone() {
            self.arm_ambient(AMBIENT_AFTER);
            return;
        }
        self.faces.ambient = true;
        // The banner on screen goes; it shows again after waking.
        self.show_banners();
        self.show_history_panel();
        if let Some(window) = self.window.upgrade() {
            window.set_ambient_drift(self.faces.drift);
            window.set_ambient_shown(true);
        }
    }

    /// Takes the idle screen away at once (a key, the pointer, a phone, or
    /// Home not alone any more).
    pub(super) fn leave_ambient(&mut self) {
        if !std::mem::take(&mut self.faces.ambient) {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            window.set_ambient_shown(false);
        }
    }

    /// Wakes Home from the idle screen: the input that woke it does
    /// nothing else, and the wait starts again.
    pub(super) fn wake_ambient(&mut self) {
        self.leave_ambient();
        self.restart_idle();
        // Banners that came during the idle screen waited for no one: they
        // are news from now, not old news (BANNER_FRESH).
        let now = (self.clock)();
        for banner in &mut self.faces.banners {
            banner.since = now;
        }
        self.show_banners();
    }

    /// After a render: the idle screen goes when Home isn't alone any more
    /// (a phone opened an app, a page opened), and banners follow.
    pub(super) fn after_render_faces(&mut self) {
        if self.faces.ambient && !self.home_alone() {
            self.leave_ambient();
        }
        self.show_banners();
    }

    /// The idle screen's setting, from the launcher's state on disk or a
    /// history overview.
    pub(super) fn set_ambient_setting(&mut self, off: bool) {
        self.faces.ambient_on = !off;
        if off {
            self.leave_ambient();
        }
    }

    /// History's summary was read again: a banner if the computer applied
    /// approvals since the last read.
    pub(super) fn history_summary_changed(&mut self, before: Option<crate::history::Summary>) {
        if let Some(banner) = panels::history_banner(before.as_ref(), self.history.0.as_ref()) {
            self.queue_banner(Panel::History, &banner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clock_text;

    #[test]
    fn the_clock_reads_hours_and_minutes() {
        // 2026-11-21 14:05:59 UTC.
        let afternoon = 1_795_270_000 - 1_795_270_000 % 86_400 + 14 * 3600 + 5 * 60 + 59;
        assert_eq!(clock_text(afternoon, true), "14:05");
        assert_eq!(clock_text(afternoon, false), "2:05");
        let midnight = afternoon - afternoon % 86_400;
        assert_eq!(clock_text(midnight, true), "00:00");
        assert_eq!(clock_text(midnight, false), "12:00");
        assert_eq!(clock_text(midnight + 12 * 3600 + 60, false), "12:01");
        assert_eq!(clock_text(midnight + 9 * 3600, true), "09:00");
        assert_eq!(clock_text(-60, true), "23:59");
    }
}
