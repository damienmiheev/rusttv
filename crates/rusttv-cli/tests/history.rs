//! End-to-end tests of `rusttv history` and of what plan-applying commands
//! tell the TV's history (HISTORY.md), against a fake TV whose door is
//! answered by the real TV side of `rusttv-history` (`rusttv-fake-tv`,
//! through `tests/fixtures/fake-adb` in scenario `history`).
//!
//! Commands that need a person at a terminal run with `RUSTTV_TEST_TERMINAL`
//! (honoured only by debug builds with `test-util`) and the typed word on
//! stdin.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers should panic on unexpected failures; clippy only exempts #[test] fns"
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const FIRST_ID: &str = "0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69";
const SECOND_ID: &str = "7d2e4f6a-1b3c-4d5e-8f9a-0b1c2d3e4f5a";

/// One fake TV and one computer's state directory.
struct Setup {
    root: PathBuf,
}

impl Setup {
    /// The Philips, launcher installed, window and animator scales stored as
    /// 1.0, `org.droidtv.gel` disabled, its history empty.
    fn new(test: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("rusttv-history-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tv")).unwrap();
        std::fs::create_dir_all(root.join("state/journals")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        let setup = Self { root };
        setup.write_device(
            r#"serialno = "PH1M0001"
launcher_installed = true
debuggable = false
granted = false
disabled = ["org.droidtv.gel"]

[global]
window_animation_scale = "1.0"
animator_duration_scale = "1.0"

[secure]
default_input_method = "com.google.android.inputmethod.latin/.LatinIME"
"#,
        );
        setup
    }

    fn tv_dir(&self) -> PathBuf {
        self.root.join("tv")
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn write_device(&self, text: &str) {
        std::fs::write(self.tv_dir().join("device.toml"), text).unwrap();
    }

    fn device(&self) -> String {
        std::fs::read_to_string(self.tv_dir().join("device.toml")).unwrap()
    }

    /// Edits `device.toml` with a text replacement.
    fn edit_device(&self, from: &str, to: &str) {
        let text = self.device();
        assert!(text.contains(from), "{from:?} not in {text}");
        self.write_device(&text.replacen(from, to, 1));
    }

    /// `rusttv <args>`; with `typed`, as at a terminal typing it.
    fn rusttv(&self, args: &[&str], typed: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusttv"));
        command
            .args(args)
            .env(
                "RUSTTV_ADB",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-adb"),
            )
            .env("FAKE_ADB_SCENARIO", "history")
            .env("FAKE_TV", env!("CARGO_BIN_EXE_rusttv-fake-tv"))
            .env("FAKE_TV_DIR", self.tv_dir())
            .env("RUSTTV_STATE_DIR", self.state())
            .env("RUSTTV_CONFIG_DIR", self.root.join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if typed.is_some() {
            command.env("RUSTTV_TEST_TERMINAL", "1");
        }
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(typed.unwrap_or_default().as_bytes())
            .unwrap();
        drop(stdin);
        child.wait_with_output().unwrap()
    }

    /// The fake TV itself: `edits <n>`, or `approve` with a change list.
    fn tv(&self, args: &[&str], stdin: &str) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rusttv-fake-tv"))
            .arg(self.tv_dir())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    /// `rusttv <args> --json` that must succeed; its `data`.
    fn ok(&self, args: &[&str], typed: Option<&str>) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.rusttv(&args, typed);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        json(&output)["data"].clone()
    }

    /// `rusttv <args> --json` that must fail; its error code and message.
    fn fails(&self, args: &[&str], typed: Option<&str>) -> (String, String) {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.rusttv(&args, typed);
        assert_ne!(output.status.code(), Some(0), "{args:?}: {output:?}");
        let error = &json(&output)["error"];
        (
            error["code"].as_str().unwrap().to_owned(),
            error["message"].as_str().unwrap().to_owned(),
        )
    }

    fn journal(&self, path: &Value) -> String {
        std::fs::read_to_string(path.as_str().unwrap()).unwrap()
    }

    fn mac_repo(&self, id: &str) -> PathBuf {
        self.state().join("history").join(format!("{id}.git"))
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn json(output: &Output) -> Value {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "rusttv.v8");
    value
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn sync_copies_the_history_first_whole_then_only_new_entries() {
    let setup = Setup::new("sync");

    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["history"], Value::Null, "{data}");
    assert_eq!(data["serialno"], "PH1M0001");
    assert_eq!(data["restore_tv"], false);

    setup.tv(&["edits", "3"], "");
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["history"], FIRST_ID);
    assert_eq!(data["new_entries"], 3);
    assert!(setup.mac_repo(FIRST_ID).join("HEAD").is_file());
    let devices = std::fs::read_to_string(setup.state().join("history/devices.toml")).unwrap();
    assert!(
        devices.contains("PH1M0001") && devices.contains(FIRST_ID),
        "{devices}"
    );

    setup.tv(&["edits", "2"], "");
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["new_entries"], 2);
    let head = data["head"].as_str().unwrap().to_owned();

    let log = setup.ok(&["history", "log", "--limit", "10"], None);
    let entries = log.as_array().unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0]["id"], head.as_str());
    assert_eq!(entries[0]["author"], "tv");

    // A prefix finds the entry; `show` lists what changed.
    let shown = setup.ok(&["history", "show", &head[..8]], None);
    assert_eq!(shown["entry"]["id"], head.as_str());
    assert_eq!(shown["changes"][0]["kind"], "reordered");
    let oldest = entries[1]["id"].as_str().unwrap();
    let diff = setup.ok(&["history", "diff", oldest], None);
    assert_eq!(diff["to"], head.as_str());
    assert_eq!(diff["changes"][0]["kind"], "reordered");
    let text = setup.rusttv(&["history", "log"], None);
    assert!(
        String::from_utf8(text.stdout).unwrap().contains("This TV"),
        "the log names who made each entry"
    );

    // Nothing new: nothing fetched.
    assert_eq!(setup.ok(&["history", "sync"], None)["new_entries"], 0);
}

#[test]
fn sync_fetches_a_long_history_in_parts() {
    let setup = Setup::new("parts");
    // More than one export part (rusttv_history::EXPORT_PART_COMMITS).
    setup.tv(&["edits", "2005"], "");
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["new_entries"], 2005, "{data}");
    let log = setup.ok(&["history", "log", "--limit", "3000"], None);
    assert_eq!(log.as_array().unwrap().len(), 2005);
}

#[test]
fn a_reinstalled_launcher_gets_its_history_back_only_while_its_history_is_empty() {
    let setup = Setup::new("restore-tv");
    setup.tv(&["edits", "2"], "");
    let first = setup.ok(&["history", "sync"], None);
    let old_head = first["head"].as_str().unwrap().to_owned();

    // The launcher's data is gone (reinstalled): this computer offers its copy.
    std::fs::remove_dir_all(setup.tv_dir().join("tv")).unwrap();
    let output = setup.rusttv(&["history", "sync", "--json"], None);
    assert_eq!(json(&output)["data"]["restore_tv"], true);
    assert!(stderr(&output).contains("rusttv history restore-tv"));

    // The dry run shows the two launcher-private steps.
    let dry = setup.ok(&["history", "restore-tv", "--dry-run"], None);
    assert_eq!(dry["plan"], "history-restore-tv");
    assert_eq!(dry["confirmation"], "history");
    assert_eq!(dry["max_risk"], "RL");
    assert_eq!(dry["steps"].as_array().unwrap().len(), 2);

    // Without a terminal nothing happens.
    let (code, _) = setup.fails(&["history", "restore-tv"], None);
    assert_eq!(code, "NOT_APPROVED");

    let report = setup.ok(&["history", "restore-tv"], Some("history\n"));
    assert_eq!(report["status"], "applied");
    let journal = setup.journal(&report["journal"]);
    assert!(journal.contains("history_inbox"), "{journal}");
    assert!(
        journal.contains("sha256"),
        "the payload is kept as its digest: {journal}"
    );

    // The TV holds the old history again, plus an `import` entry.
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["history"], FIRST_ID);
    assert_eq!(data["new_entries"], 1);
    let log = setup.ok(&["history", "log"], None);
    assert_eq!(log[0]["author"], "import");
    assert_eq!(log[1]["id"], old_head.as_str());

    // Never into a history that isn't empty.
    let (code, message) = setup.fails(&["history", "restore-tv"], Some("history\n"));
    assert_eq!(code, "HISTORY_NOT_EMPTY", "{message}");
}

#[test]
fn a_new_history_on_the_tv_is_fetched_whole_next_to_the_old_one() {
    let setup = Setup::new("new-history");
    setup.tv(&["edits", "2"], "");
    setup.ok(&["history", "sync"], None);

    // Reinstalled and something recorded on the TV first: a new history,
    // which doesn't hold this computer's newest entry (HISTORY_NOT_FOUND).
    std::fs::remove_dir_all(setup.tv_dir().join("tv")).unwrap();
    std::fs::write(setup.tv_dir().join("new-id"), SECOND_ID).unwrap();
    setup.tv(&["edits", "1"], "");
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["history"], SECOND_ID);
    assert_eq!(data["new_entries"], 1);
    assert_eq!(data["restore_tv"], false);
    assert!(setup.mac_repo(FIRST_ID).join("HEAD").is_file());
    // The old one stays readable here.
    let old = setup.ok(&["history", "log", "--history", FIRST_ID], None);
    assert_eq!(old.as_array().unwrap().len(), 2);
    // And restore-tv now needs --history.
    let (code, _) = setup.fails(&["history", "restore-tv", "--dry-run"], None);
    assert_eq!(code, "HISTORY_AMBIGUOUS");
}

/// Approves `changes` on the fake TV, as a person with the remote would.
fn approve_on_tv(setup: &Setup, changes: &[(&str, &str, &str)]) {
    let list: String = changes
        .iter()
        .map(|(package, old, new)| {
            format!(
                "[[change]]\nkind = \"package\"\nname = \"{package}\"\nold = \"{old}\"\nnew = \"{new}\"\n"
            )
        })
        .collect();
    setup.tv(&["approve"], &list);
}

#[test]
fn apply_pending_applies_exactly_what_was_approved_on_the_tv_once() {
    let setup = Setup::new("pending");
    setup.tv(&["edits", "1"], "");
    approve_on_tv(&setup, &[("org.droidtv.candeebug", "enabled", "disabled")]);

    // `sync` mentions it, with the time left.
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["pending"][0]["seconds_left"], 7200);

    // Never without a terminal: nothing consumed.
    let (code, _) = setup.fails(&["history", "apply-pending"], None);
    assert_eq!(code, "NOT_APPROVED");
    assert_eq!(
        setup.ok(&["history", "sync"], None)["pending"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Nothing to type: the plan is exactly what was approved.
    let reports = setup.ok(&["history", "apply-pending"], Some(""));
    assert_eq!(reports[0]["approved_on_tv"], true, "{reports}");
    assert_eq!(reports[0]["status"], "applied");
    assert!(setup.device().contains("org.droidtv.candeebug"));
    let journal = setup.journal(&reports[0]["journal"]);
    assert!(journal.contains("by = \"tv\""), "{journal}");
    assert!(journal.contains("disable_package"), "{journal}");

    // The TV recorded the note with its approval.
    setup.ok(&["history", "sync"], None);
    let log = setup.ok(&["history", "log"], None);
    assert_eq!(log[0]["author"], "mac");
    assert!(
        log[0]["approved_by"]
            .as_str()
            .unwrap()
            .starts_with("tv-code "),
        "{log}"
    );

    // Used once: nothing waits any more.
    let reports = setup.ok(&["history", "apply-pending"], Some(""));
    assert_eq!(reports.as_array().unwrap().len(), 0);
}

#[test]
fn apply_pending_asks_for_the_typed_word_when_the_device_changed_since() {
    let setup = Setup::new("pending-mismatch");
    setup.tv(&["edits", "1"], "");
    approve_on_tv(
        &setup,
        &[
            ("org.droidtv.candeebug", "enabled", "disabled"),
            ("com.google.android.partnersetup", "enabled", "disabled"),
        ],
    );
    // Disabled meanwhile by someone else: the plan has one step, not two.
    setup.edit_device(
        "disabled = [\"org.droidtv.gel\"]",
        "disabled = [\"com.google.android.partnersetup\", \"org.droidtv.gel\"]",
    );

    // Anything but the word cancels (the approval is used up regardless).
    let (code, _) = setup.fails(&["history", "apply-pending"], Some("no\n"));
    assert_eq!(code, "NOT_APPROVED");
    assert!(!setup.device().contains("org.droidtv.candeebug"));

    approve_on_tv(
        &setup,
        &[
            ("org.droidtv.candeebug", "enabled", "disabled"),
            ("com.android.tv.feedbackconsent", "enabled", "disabled"),
        ],
    );
    setup.edit_device(
        "disabled = [",
        "disabled = [\"com.android.tv.feedbackconsent\", ",
    );
    let output = setup.rusttv(&["history", "apply-pending", "--json"], Some("restore\n"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let reports = &json(&output)["data"];
    assert_eq!(reports[0]["approved_on_tv"], false);
    assert!(stderr(&output).contains("isn't exactly what was approved on the TV"));
    assert!(setup.device().contains("org.droidtv.candeebug"));
    let journal = setup.journal(&reports[0]["journal"]);
    assert!(journal.contains("by = \"terminal\""), "{journal}");
}

#[test]
fn apply_pending_refuses_approvals_from_a_test_build() {
    let setup = Setup::new("pending-debuggable");
    setup.tv(&["edits", "1"], "");
    approve_on_tv(&setup, &[("org.droidtv.candeebug", "enabled", "disabled")]);
    setup.edit_device("debuggable = false", "debuggable = true");
    let (code, _) = setup.fails(&["history", "apply-pending"], Some(""));
    assert_eq!(code, "PENDING_REFUSED");
    assert!(!setup.device().contains("org.droidtv.candeebug"));
}

#[test]
fn plans_are_observed_before_and_noted_after_and_snapshots_restore_through_plans() {
    let setup = Setup::new("restore");
    setup.tv(&["edits", "1"], "");
    let before = setup.ok(&["history", "sync"], None)["head"]
        .as_str()
        .unwrap()
        .to_owned();

    // A plan-applying command: an observation before, a note after.
    let applied = setup.ok(
        &["optimize", "apply", "reduced-animations"],
        Some("optimize\n"),
    );
    assert_eq!(applied["status"], "applied");
    assert!(setup.device().contains("window_animation_scale = \"0.5\""));
    let log = setup.ok(&["history", "log"], None);
    assert_eq!(log[0]["author"], "mac", "{log}");
    assert_eq!(log[0]["approved_by"], "terminal");
    let journal_name = Path::new(applied["journal"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(log[0]["journal"], journal_name.as_str());
    let shown = setup.ok(&["history", "show", log[0]["id"].as_str().unwrap()], None);
    assert_eq!(
        shown["state"]["settings"]["global"]["window_animation_scale"],
        "0.5"
    );

    // Restoring the entry before conflicts with the optimizer journal.
    let (code, message) = setup.fails(&["history", "restore", &before, "--dry-run"], None);
    assert_eq!(code, "PRECONDITION_FAILED");
    assert!(
        message.contains("rusttv optimize restore --journal"),
        "{message}"
    );

    // Undo the optimizer change; then restore the entry with 0.5.
    setup.ok(&["optimize", "restore"], Some("restore\n"));
    assert!(setup.device().contains("window_animation_scale = \"1.0\""));
    let half = log[0]["id"].as_str().unwrap().to_owned();
    let dry = setup.ok(&["history", "restore", &half, "--dry-run"], None);
    assert_eq!(dry["plan"], "history-restore");
    assert_eq!(dry["confirmation"], "restore");
    let restored = setup.ok(&["history", "restore", &half], Some("restore\n"));
    assert_eq!(restored["status"], "applied");
    assert!(setup.device().contains("window_animation_scale = \"0.5\""));

    // Its journal undoes it.
    let path = restored["journal"].as_str().unwrap().to_owned();
    let undone = setup.ok(
        &["history", "restore", "--journal", &path],
        Some("restore\n"),
    );
    assert_eq!(undone["status"], "restored");
    assert!(setup.device().contains("window_animation_scale = \"1.0\""));
    // Nothing left to undo now.
    let (code, _) = setup.fails(
        &["history", "restore", "--journal", &path, "--dry-run"],
        None,
    );
    assert_eq!(code, "PRECONDITION_FAILED");
}

#[test]
fn nothing_is_sent_into_an_empty_tv_history_while_this_computer_holds_one() {
    let setup = Setup::new("empty-rule");
    setup.tv(&["edits", "1"], "");
    setup.ok(&["history", "sync"], None);
    std::fs::remove_dir_all(setup.tv_dir().join("tv")).unwrap();

    let output = setup.rusttv(&["history", "allow-tv-settings", "--json"], Some("allow\n"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(setup.device().contains("granted = true"));
    assert!(stderr(&output).contains("rusttv history restore-tv"));
    // Still empty: restore-tv can still put the old one back.
    let data = setup.ok(&["history", "sync"], None);
    assert_eq!(data["history"], Value::Null);
    assert_eq!(data["restore_tv"], true);

    // And the grant is revoked the same way.
    setup.ok(&["history", "revoke-tv-settings"], Some("revoke\n"));
    assert!(setup.device().contains("granted = false"));
}

#[test]
fn sync_warns_about_watched_settings_changed_since_the_last_one() {
    let setup = Setup::new("watchlist");
    setup.tv(&["edits", "1"], "");
    assert_eq!(
        setup.ok(&["history", "sync"], None)["watchlist"],
        serde_json::json!([])
    );
    setup.edit_device(
        "[secure]",
        "[secure]\nenabled_accessibility_services = \"com.evil/.Spy\"",
    );
    let output = setup.rusttv(&["history", "sync", "--json"], None);
    let changes = &json(&output)["data"]["watchlist"];
    assert_eq!(changes[0]["key"], "secure enabled_accessibility_services");
    assert_eq!(changes[0]["before"], Value::Null);
    assert_eq!(changes[0]["now"], "com.evil/.Spy");
    assert!(stderr(&output).contains("warning: secure enabled_accessibility_services"));
}

#[test]
fn uninstalling_the_launcher_copies_its_history_first() {
    let setup = Setup::new("uninstall");
    setup.tv(&["edits", "3"], "");
    let output = setup.rusttv(&["launcher", "uninstall", "--json"], Some("uninstall\n"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stderr(&output).contains("Synced the TV's history"));
    assert!(setup.device().contains("launcher_installed = false"));
    let log = setup.ok(&["history", "log", "--history", FIRST_ID], None);
    assert_eq!(log.as_array().unwrap().len(), 3);
}

#[test]
fn history_errors_have_codes() {
    let setup = Setup::new("errors");
    let (code, _) = setup.fails(&["history", "log"], None);
    assert_eq!(code, "HISTORY_NOT_FOUND");
    let (code, _) = setup.fails(&["history", "log", "--history", "not-a-uuid"], None);
    assert_eq!(code, "HISTORY_NOT_FOUND");
    let (code, _) = setup.fails(&["history", "restore-tv", "--dry-run"], None);
    assert_eq!(code, "HISTORY_NOT_FOUND");
}
