//! The apps the launcher started and keeps running: one instance per app,
//! at most [`Limits::max_apps`] at once, and background apps closed when
//! memory runs low. The least recently used app closes first. Apps in the
//! background can be frozen (SIGSTOP), like Android's cached-app freezer:
//! they keep their memory but take no CPU or GPU time from the app in front.

use std::fs;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

/// How many apps run and how much memory they leave.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Apps kept running besides the launcher; starting one more closes the
    /// least recently used. `None`: no limit.
    pub max_apps: Option<usize>,
    /// Close background apps (least recently used first) while the kernel's
    /// `MemAvailable` is below this many MiB. `None`: never.
    pub min_available_mib: Option<u64>,
    /// Stop (SIGSTOP) every app but the one in front, and continue it when
    /// its window comes to the front again. Also pauses background music.
    pub freeze_background: bool,
}

/// How long an app has to quit after SIGTERM before it gets SIGKILL.
const GRACE: Duration = Duration::from_secs(5);

/// How often available memory is checked.
const MEMORY_POLL: Duration = Duration::from_secs(2);

/// The running apps, shared with the threads that wait for them.
#[derive(Debug, Default)]
pub struct Tasks {
    limits: Limits,
    running: Mutex<Vec<Task>>,
    /// The app whose window was last in front (not the launcher's).
    front: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
struct Task {
    id: String,
    /// The app's process group (its first process's PID).
    group: i32,
    /// Lower-case app IDs its windows may have.
    keys: Vec<String>,
    /// Reads game controllers itself.
    reads_pads: bool,
    used: Instant,
    closing: bool,
    frozen: bool,
}

impl Tasks {
    /// Starts watching memory if `limits` asks for it.
    pub fn start(limits: Limits) -> Arc<Self> {
        let tasks = Arc::new(Self {
            limits,
            running: Mutex::default(),
            front: Mutex::default(),
        });
        if let Some(min) = limits.min_available_mib {
            let weak = Arc::downgrade(&tasks);
            let spawned = std::thread::Builder::new()
                .name("rusttv-memory".into())
                .spawn(move || {
                    while let Some(tasks) = weak.upgrade() {
                        if available_mib().is_some_and(|available| available < min) {
                            tasks.close_one_in_background("memory is low");
                        }
                        drop(tasks);
                        std::thread::sleep(MEMORY_POLL);
                    }
                });
            if let Err(error) = spawned {
                log::warn!("no thread to watch memory: {error}");
            }
        }
        tasks
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Task>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The running app whose window was last in front of the launcher.
    pub fn front(&self) -> Option<String> {
        let front = self
            .front
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        self.is_running(&front).then_some(front)
    }

    /// Closes app `id` if it runs.
    pub fn close_app(&self, id: &str) {
        let task = {
            let mut running = self.lock();
            let Some(task) = running
                .iter_mut()
                .find(|task| task.id == id && !task.closing)
            else {
                return;
            };
            task.closing = true;
            task.clone()
        };
        log::info!("closing {id} (asked for)");
        self.close(task);
    }

    /// Whether `id` runs (and is not being closed).
    pub fn is_running(&self, id: &str) -> bool {
        self.lock()
            .iter()
            .any(|task| task.id == id && !task.closing)
    }

    /// The window keys of `id` if it is running (and not being closed); marks
    /// it as just used.
    pub fn reuse(&self, id: &str) -> Option<Vec<String>> {
        let mut running = self.lock();
        let task = running
            .iter_mut()
            .find(|task| task.id == id && !task.closing)?;
        task.used = Instant::now();
        if task.frozen {
            task.frozen = false;
            signal(task.group, Signal::CONT);
        }
        Some(task.keys.clone())
    }

    /// Records a started app, then closes the least recently used others
    /// beyond [`Limits::max_apps`].
    pub fn started(&self, id: &str, group: u32, keys: Vec<String>, reads_pads: bool) {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        let victims: Vec<Task> = {
            let mut running = self.lock();
            running.push(Task {
                id: id.to_owned(),
                group,
                keys,
                reads_pads,
                used: Instant::now(),
                closing: false,
                frozen: false,
            });
            let Some(max) = self.limits.max_apps else {
                return;
            };
            let mut open: Vec<&mut Task> = running
                .iter_mut()
                .filter(|task| !task.closing && task.group != group)
                .collect();
            open.sort_by_key(|task| task.used);
            let excess = (open.len() + 1).saturating_sub(max.max(1));
            open.into_iter()
                .take(excess)
                .map(|task| {
                    task.closing = true;
                    task.clone()
                })
                .collect()
        };
        for task in victims {
            log::info!(
                "closing {} (more than {:?} apps)",
                task.id,
                self.limits.max_apps
            );
            self.close(task);
        }
    }

    /// The app in process group `group` ended.
    pub fn ended(&self, group: u32) {
        let Ok(group) = i32::try_from(group) else {
            return;
        };
        self.lock().retain(|task| task.group != group);
    }

    /// A window with this app ID came to the front (the launcher's own too):
    /// its app was just used, and with [`Limits::freeze_background`] every
    /// other app stops. Returns whether that app reads game controllers
    /// itself; `None` for a window of no app started here.
    pub fn activated(&self, app_id: &str) -> Option<bool> {
        let app_id = app_id.to_lowercase();
        let mut front_id = None;
        for task in self.lock().iter_mut() {
            let front = task.keys.contains(&app_id);
            if front {
                task.used = Instant::now();
                front_id = Some((task.id.clone(), task.reads_pads));
            }
            if front && task.frozen {
                task.frozen = false;
                signal(task.group, Signal::CONT);
            } else if !front && !task.frozen && !task.closing && self.limits.freeze_background {
                task.frozen = true;
                log::info!("freezing {} in the background", task.id);
                signal(task.group, Signal::STOP);
            }
        }
        let (id, reads_pads) = front_id?;
        *self.front.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
        Some(reads_pads)
    }

    /// Closes the least recently used app that is not the most recent one.
    fn close_one_in_background(&self, reason: &str) {
        let victim = {
            let mut running = self.lock();
            let mut open: Vec<&mut Task> =
                running.iter_mut().filter(|task| !task.closing).collect();
            if open.len() < 2 {
                return;
            }
            open.sort_by_key(|task| task.used);
            let task = &mut open[0];
            task.closing = true;
            task.clone()
        };
        log::warn!("closing {} ({reason})", victim.id);
        self.close(victim);
    }

    /// SIGTERM to the app's process group; SIGKILL if it is still there
    /// after [`GRACE`].
    fn close(&self, task: Task) {
        let Some(pid) = Pid::from_raw(task.group) else {
            return;
        };
        if let Err(error) = kill_process_group(pid, Signal::TERM) {
            log::warn!("stopping {} failed: {error}", task.id);
            return;
        }
        // A frozen app must run to handle SIGTERM.
        signal(task.group, Signal::CONT);
        let name = task.id.clone();
        let spawned = std::thread::Builder::new()
            .name("rusttv-app-close".into())
            .spawn(move || {
                std::thread::sleep(GRACE);
                if test_kill_process_group(pid).is_ok() {
                    log::warn!("{} ignored SIGTERM; killing it", task.id);
                    let _ = kill_process_group(pid, Signal::KILL);
                }
            });
        if let Err(error) = spawned {
            log::warn!("no thread to finish closing {name}: {error}");
        }
    }
}

/// The process group of a running `program` (a file name) of this user that
/// leads a group of its own, as apps the launcher starts do: an app left
/// running by an earlier launcher. The program is the first argument, or
/// the second for an interpreter running a script.
pub fn find_group(program: &str) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    let uid = rustix::process::getuid().as_raw();
    let me = std::process::id();
    let mut found: Vec<u32> = fs::read_dir("/proc")
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            if pid == me || entry.metadata().ok()?.uid() != uid {
                return None;
            }
            let dir = entry.path();
            let stat = fs::read_to_string(dir.join("stat")).ok()?;
            let cmdline = fs::read(dir.join("cmdline")).ok()?;
            (leads_group(&stat, pid) && runs(&cmdline, program)).then_some(pid)
        })
        .collect();
    // The oldest, should there be more.
    found.sort_unstable();
    found.first().copied()
}

/// Whether `/proc/<pid>/stat` says `pid` leads its process group.
fn leads_group(stat: &str, pid: u32) -> bool {
    // "pid (comm) state ppid pgrp …"; comm may hold spaces and parentheses.
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(2))
        .and_then(|group| group.parse::<u32>().ok())
        == Some(pid)
}

/// Whether a `/proc/<pid>/cmdline` runs `program`.
fn runs(cmdline: &[u8], program: &str) -> bool {
    let name = |arg: &[u8]| {
        std::path::Path::new(&*String::from_utf8_lossy(arg))
            .file_name()
            .is_some_and(|name| name == program)
    };
    let mut args = cmdline.split(|&byte| byte == 0);
    args.next().is_some_and(name) || args.next().is_some_and(name)
}

/// Sends `sig` to process group `group`; logs a failure.
fn signal(group: i32, sig: Signal) {
    let Some(pid) = Pid::from_raw(group) else {
        return;
    };
    if let Err(error) = kill_process_group(pid, sig) {
        log::warn!("signal {sig:?} to process group {group} failed: {error}");
    }
}

/// `MemAvailable` from `/proc/meminfo`, in MiB.
fn available_mib() -> Option<u64> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_apps_by_program_and_group() {
        assert!(leads_group("23840 (dolphin) S 1 23840 23840 0", 23840));
        assert!(leads_group("77 (odd ) name) S 1 77 77", 77));
        assert!(!leads_group("23841 (dolphin) S 23840 23840 23840", 23841));
        assert!(runs(b"dolphin\0", "dolphin"));
        assert!(runs(b"/usr/bin/dolphin\0--new-window\0", "dolphin"));
        // A wrapper script under its interpreter.
        assert!(runs(b"/bin/sh\0/usr/bin/kodi\0", "kodi"));
        assert!(!runs(b"/usr/bin/dolphin-helper\0", "dolphin"));
        assert!(!runs(b"", "dolphin"));
    }

    #[test]
    fn reuses_a_running_app_and_forgets_it_when_it_ends() {
        let tasks = Tasks::start(Limits::default());
        assert_eq!(tasks.reuse("kodi.desktop"), None);
        tasks.started("kodi.desktop", 4_000_000, vec!["kodi".into()], true);
        assert_eq!(tasks.reuse("kodi.desktop"), Some(vec!["kodi".to_owned()]));
        tasks.ended(4_000_000);
        assert_eq!(tasks.reuse("kodi.desktop"), None);
    }

    #[test]
    fn remembers_the_app_last_in_front_but_not_the_launcher() {
        let limits = Limits {
            freeze_background: false,
            ..Limits::default()
        };
        let tasks = Tasks::start(limits);
        assert_eq!(tasks.front(), None);
        tasks.started(
            "org.kde.dolphin.desktop",
            4_000_001,
            vec!["org.kde.dolphin".into()],
            false,
        );
        assert_eq!(tasks.activated("org.kde.dolphin"), Some(false));
        // The launcher's own window comes to the front: Dolphin stays the one.
        assert_eq!(tasks.activated(""), None);
        assert_eq!(tasks.front().as_deref(), Some("org.kde.dolphin.desktop"));
        tasks.ended(4_000_001);
        assert_eq!(tasks.front(), None);
    }
}
