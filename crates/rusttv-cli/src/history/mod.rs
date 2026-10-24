//! `rusttv history ...` (HISTORY.md): this computer's copy of each TV's
//! history, restoring snapshots through plans, putting the history back on
//! the TV, approvals made on the TV, and the launcher's settings grant.

mod door;
mod mac;
pub(crate) mod record;
mod sync;
mod words;

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use rusttv_core::{
    Adb, FileJournal, HISTORY_RESTORE, HISTORY_RESTORE_TV, HistorySince, JournalStatus,
    JournalStore, PackageName, PlanError, RestoreTarget, SettingKey, SettingState, SettingValue,
    TvChange,
};
use rusttv_history::door::{ApprovedChange, ExportHeader, Message, PendingApproval};
use rusttv_history::{EntryId, HistoryId, PackageState, State, Store};
use serde::Serialize;

use crate::launcher::{
    ChangeReport, apply_recorded, confirm, read_confirmation, restore_journal, show_plan,
};
use crate::{CliError, Target, output};

pub(crate) use record::Recorder;

/// A history command failed; `code` is its stable error code (CLI.md).
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct Failure {
    pub(crate) code: &'static str,
    message: String,
}

impl Failure {
    pub(crate) fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    /// The TV or this computer's copy holds something that breaks the format.
    pub(crate) fn invalid(message: String) -> Self {
        Self::new("HISTORY_INVALID", message)
    }

    /// A file of this computer's copy can't be read or written.
    pub(crate) fn storage(path: &Path, error: &std::io::Error) -> Self {
        Self::new("HISTORY_STORAGE", format!("{}: {error}", path.display()))
    }

    /// The TV's door refused `what`, with its code.
    pub(crate) fn refused(code: &str, what: &str) -> Self {
        let hint = match code {
            "TV_BUSY" => ": a change on the TV is in progress; try again in a moment",
            "HISTORY_RATE_LIMITED" => ": too many entries from this computer this hour",
            "HISTORY_FORMAT_NEWER" => {
                ": the TV's history is from a newer RustTV; run `rusttv update cli`"
            }
            "PENDING_INVALID" => ": the approval was used already or has expired",
            "HISTORY_NOT_EMPTY" => ": the TV already has a history",
            _ => "",
        };
        let error = if code == "HISTORY_FORMAT_NEWER" {
            "HISTORY_FORMAT_NEWER"
        } else {
            "HISTORY_REFUSED"
        };
        Self::new(error, format!("the TV refused {what} ({code}){hint}"))
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum HistoryAction {
    /// Fetch the TV's new history entries into this computer's copy, show
    /// the approvals waiting on the TV, and warn about sensitive settings
    /// that changed since the last sync. Read-only on the TV.
    Sync {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
    },
    /// List entries of this computer's copy, newest first.
    Log {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
        /// How many entries.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// One entry: who, when, what changed, and the whole snapshot.
    Show {
        /// Entry id, or a unique prefix of it (at least 4 hex digits).
        entry: String,
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
    },
    /// What changed from one entry to another (default: the newest).
    Diff {
        /// From this entry (id or unique prefix).
        from: String,
        /// To this entry; the newest if left out.
        to: Option<String>,
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
    },
    /// Put the device's settings and catalog packages back as an entry
    /// recorded them, or (--journal) undo such a restore. Shows the plan and
    /// asks you to type a confirmation; rolls back on failure.
    Restore {
        /// Entry id, or a unique prefix of it.
        #[arg(required_unless_present = "journal", conflicts_with = "journal")]
        entry: Option<String>,
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
        /// Undo the `history restore` recorded in this journal instead.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Also plan disabling packages not tested on this model; the plan
        /// marks each one UNTESTED.
        #[arg(long, conflicts_with = "journal")]
        allow_untested: bool,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Put this computer's copy of the TV's history back on the TV, e.g.
    /// after reinstalling the launcher; only while the TV's history is empty.
    RestoreTv {
        #[command(flatten)]
        target: Target,
        #[command(flatten)]
        history: Which,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply the changes approved on the TV that wait for this computer
    /// (packages, and settings without the grant). Interactive terminal
    /// only; nothing to type when the plan is exactly what was approved.
    ApplyPending {
        #[command(flatten)]
        target: Target,
    },
    /// Let the launcher change its animation settings itself, after you
    /// approve each change on the TV (grants WRITE_SECURE_SETTINGS).
    AllowTvSettings {
        #[command(flatten)]
        target: Target,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Take that permission back.
    RevokeTvSettings {
        #[command(flatten)]
        target: Target,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Which history on this computer.
#[derive(Debug, Args)]
pub(crate) struct Which {
    /// History id (a UUID); needed when the TV reports no serial number, or
    /// this computer holds several histories for it.
    #[arg(long = "history")]
    id: Option<String>,
}

impl Which {
    fn id(&self) -> Result<Option<HistoryId>, CliError> {
        self.id
            .as_deref()
            .map(|id| {
                HistoryId::parse(id).map_err(|_| {
                    Failure::new(
                        "HISTORY_NOT_FOUND",
                        format!("`{id}` is not a history id (a UUID)"),
                    )
                    .into()
                })
            })
            .transpose()
    }
}

/// Runs a `history` command.
pub(crate) fn run(adb: &dyn Adb, action: &HistoryAction, json: bool) -> Result<(), CliError> {
    match action {
        HistoryAction::Sync { target, history } => sync_command(adb, target, history, json),
        HistoryAction::Log {
            target,
            history,
            limit,
        } => log(adb, target, history, *limit, json),
        HistoryAction::Show {
            entry,
            target,
            history,
        } => show(adb, target, history, entry, json),
        HistoryAction::Diff {
            from,
            to,
            target,
            history,
        } => diff(adb, target, history, from, to.as_deref(), json),
        HistoryAction::Restore {
            entry,
            target,
            history,
            journal,
            allow_untested,
            dry_run,
        } => match (entry, journal) {
            (_, Some(journal)) => restore_from_journal(adb, target, journal, *dry_run, json),
            (Some(entry), None) => {
                restore(adb, target, history, entry, *allow_untested, *dry_run, json)
            }
            (None, None) => Err(Failure::new(
                "HISTORY_NOT_FOUND",
                "name an entry, or pass --journal".to_owned(),
            )
            .into()),
        },
        HistoryAction::RestoreTv {
            target,
            history,
            dry_run,
        } => restore_tv(adb, target, history, *dry_run, json),
        HistoryAction::ApplyPending { target } => apply_pending(adb, target, json),
        HistoryAction::AllowTvSettings { target, dry_run } => {
            tv_settings(adb, target, true, *dry_run, json)
        }
        HistoryAction::RevokeTvSettings { target, dry_run } => {
            tv_settings(adb, target, false, *dry_run, json)
        }
    }
}

/// `history sync` in `--json` output.
#[derive(Debug, Serialize)]
struct SyncReport {
    serial: String,
    /// The TV's `ro.serialno`, or null when it reports none usable.
    serialno: Option<String>,
    /// The TV's history id; null while its history is empty.
    history: Option<String>,
    /// The newest entry this computer has now.
    head: Option<String>,
    /// Entries fetched by this sync.
    new_entries: usize,
    /// The TV's history is empty while this computer holds one for it:
    /// `history restore-tv` can put it back.
    restore_tv: bool,
    /// Approvals made on the TV that wait for this computer.
    pending: Vec<PendingView>,
    /// Watchlist values changed since the last sync.
    watchlist: Vec<sync::WatchChange>,
}

/// A pending approval in `--json` output.
#[derive(Debug, Serialize)]
struct PendingView {
    approval: String,
    seconds_left: u64,
    changes: Vec<String>,
}

impl PendingView {
    fn new(approval: &PendingApproval) -> Self {
        Self {
            approval: approval.hash.clone(),
            seconds_left: approval.seconds_left,
            changes: approval.change.iter().map(approved_text).collect(),
        }
    }
}

fn approved_text(change: &ApprovedChange) -> String {
    let setting = |value: &Option<String>| value.clone().unwrap_or_else(|| "unset".to_owned());
    match change {
        ApprovedChange::Setting { key, old, new } => {
            format!("{key}: {} → {}", setting(old), setting(new))
        }
        ApprovedChange::Package { name, old, new } => format!(
            "{name}: {} → {new}",
            old.map_or_else(|| "not seen".to_owned(), |old| old.to_string())
        ),
    }
}

fn sync_command(adb: &dyn Adb, target: &Target, which: &Which, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let forced = which.id()?;
    let synced = sync::sync(adb, &serial, forced.as_ref())?;
    let pending = if synced.history.is_some() {
        door::pending(adb, &serial)?.approval
    } else {
        Vec::new()
    };
    let watchlist = match &synced.history {
        Some(id) => sync::watchlist(adb, &serial, id)?,
        None => Vec::new(),
    };
    let report = SyncReport {
        serial,
        serialno: synced.serialno.clone(),
        history: synced.history.as_ref().map(ToString::to_string),
        head: synced.head.as_ref().map(ToString::to_string),
        new_entries: synced.new_entries,
        restore_tv: synced.restore_tv_offered(),
        pending: pending.iter().map(PendingView::new).collect(),
        watchlist,
    };
    for change in &report.watchlist {
        let shown = |value: &Option<String>| value.clone().unwrap_or_else(|| "unset".to_owned());
        output::print_note(&format!(
            "warning: {} changed since the last sync: {} → {}{}",
            change.key,
            shown(&change.before),
            shown(&change.now),
            if change.key == "write_secure_settings" && change.now.as_deref() != Some("granted") {
                " (reinstalling the launcher drops it: `rusttv history allow-tv-settings`)"
            } else {
                ""
            }
        ));
    }
    if report.restore_tv {
        record::offer_restore_tv(synced.mac_histories.len());
    }
    if json {
        output::print_json(&report);
        return Ok(());
    }
    match (&report.history, synced.serialno.is_none()) {
        (Some(id), _) => {
            println!(
                "History {id}: {} new entr{}; newest {}.",
                report.new_entries,
                if report.new_entries == 1 { "y" } else { "ies" },
                report.head.as_deref().map_or("none", |head| &head[..12])
            );
            if let Ok(dir) = mac::dir() {
                println!("Copy: {}", dir.join(format!("{id}.git")).display());
            }
        }
        (None, _) => println!("The TV's history is empty."),
    }
    if synced.serialno.is_none() {
        println!(
            "The TV reports no serial number, so this copy isn't linked to it: pass --history \
             <id> to the other history commands."
        );
    }
    for approval in &report.pending {
        println!(
            "\nWaiting for this computer (approved on the TV), {} left: run `rusttv history \
             apply-pending`.",
            words::duration(approval.seconds_left)
        );
        for change in &approval.changes {
            println!("  {change}");
        }
    }
    Ok(())
}

/// This computer's copy of a history: `--history`, or the newest one
/// linked to the device's `ro.serialno`.
fn mac_history(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
) -> Result<(HistoryId, Store), CliError> {
    if let Some(id) = which.id()? {
        let store = mac::existing(&id)?.ok_or_else(|| {
            Failure::new(
                "HISTORY_NOT_FOUND",
                format!("this computer has no copy of history {id}"),
            )
        })?;
        return Ok((id, store));
    }
    let serial = target.resolve(adb)?;
    let Some(serialno) = mac::serialno(adb, &serial)? else {
        return Err(Failure::new(
            "HISTORY_NOT_FOUND",
            format!(
                "the TV reports no serial number, so no history is linked to it; pass --history \
                 <id> (the copies are in {})",
                mac::dir()?.display()
            ),
        )
        .into());
    };
    for id in mac::Devices::load()?.histories(&serialno).iter().rev() {
        if let Some(store) = mac::existing(id)? {
            return Ok((id.clone(), store));
        }
    }
    Err(Failure::new(
        "HISTORY_NOT_FOUND",
        "this computer has no copy of this TV's history yet; run `rusttv history sync`".to_owned(),
    )
    .into())
}

/// An entry by id or unique prefix (at least 4 hex digits).
fn entry_id(store: &Store, text: &str) -> Result<EntryId, CliError> {
    if let Ok(id) = EntryId::parse(text) {
        store.entry(&id).map_err(Failure::from)?;
        return Ok(id);
    }
    let text = text.to_ascii_lowercase();
    if text.len() < 4 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Failure::new(
            "HISTORY_NOT_FOUND",
            format!("`{text}` is not an entry id or a prefix of at least 4 hex digits"),
        )
        .into());
    }
    let found: Vec<EntryId> = store
        .log(usize::MAX)
        .map_err(Failure::from)?
        .into_iter()
        .map(|entry| entry.id)
        .filter(|id| id.as_str().starts_with(&text))
        .collect();
    match found.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(Failure::new("HISTORY_NOT_FOUND", format!("no entry {text}")).into()),
        _ => Err(Failure::new(
            "HISTORY_NOT_FOUND",
            format!("{text} matches several entries; give more digits"),
        )
        .into()),
    }
}

fn log(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
    limit: usize,
    json: bool,
) -> Result<(), CliError> {
    let (id, store) = mac_history(adb, target, which)?;
    let entries = store.log(limit).map_err(Failure::from)?;
    if json {
        let views: Vec<words::EntryView> = entries.iter().map(words::EntryView::new).collect();
        output::print_json(&views);
        return Ok(());
    }
    println!("History {id} (this computer's copy; `rusttv history sync` fetches new entries)");
    if entries.is_empty() {
        println!("No entries.");
    }
    for entry in &entries {
        println!("{}", words::entry_line(entry));
    }
    Ok(())
}

/// `history show` in `--json` output.
#[derive(Debug, Serialize)]
struct ShowReport {
    entry: words::EntryView,
    /// What changed from the entry before (everything, for the first).
    changes: Vec<words::ChangeView>,
    /// The snapshot.
    state: State,
}

fn show(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
    text: &str,
    json: bool,
) -> Result<(), CliError> {
    let (_, store) = mac_history(adb, target, which)?;
    let id = entry_id(&store, text)?;
    let entry = store.entry(&id).map_err(Failure::from)?;
    let state = store.state(&id).map_err(Failure::from)?;
    let before = match &entry.parent {
        Some(parent) => store.state(parent).map_err(Failure::from)?,
        None => State::new(),
    };
    let changes = rusttv_history::diff(&before, &state);
    if json {
        output::print_json(&ShowReport {
            entry: words::EntryView::new(&entry),
            changes: changes.iter().filter_map(words::ChangeView::new).collect(),
            state,
        });
        return Ok(());
    }
    println!("Entry   {}", entry.id);
    println!("Time    {}", words::time(entry.time_unix).trim_end());
    println!("Who     {}", words::who(entry.meta.author));
    if let Some(journal) = &entry.meta.journal {
        println!("Journal {journal}");
    }
    println!("\n{}\n", entry.summary);
    for change in &changes {
        println!("  {}", words::change(change));
    }
    let text = state.to_toml().map_err(Failure::from)?;
    println!("\nstate.toml:\n{text}");
    Ok(())
}

/// `history diff` in `--json` output.
#[derive(Debug, Serialize)]
struct DiffReport {
    from: String,
    to: String,
    changes: Vec<words::ChangeView>,
}

fn diff(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
    from: &str,
    to: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let (_, store) = mac_history(adb, target, which)?;
    let from = entry_id(&store, from)?;
    let to = match to {
        Some(to) => entry_id(&store, to)?,
        None => store
            .head()
            .map_err(Failure::from)?
            .ok_or_else(|| Failure::new("HISTORY_NOT_FOUND", "the history is empty".to_owned()))?,
    };
    let changes = rusttv_history::diff(
        &store.state(&from).map_err(Failure::from)?,
        &store.state(&to).map_err(Failure::from)?,
    );
    if json {
        output::print_json(&DiffReport {
            from: from.to_string(),
            to: to.to_string(),
            changes: changes.iter().filter_map(words::ChangeView::new).collect(),
        });
        return Ok(());
    }
    println!("From {} to {}:", &from.as_str()[..12], &to.as_str()[..12]);
    if changes.is_empty() {
        println!("  nothing changed");
    }
    for change in &changes {
        println!("  {}", words::change(change));
    }
    Ok(())
}

/// What a snapshot says about settings and catalog packages.
fn target_of(state: &State) -> Result<RestoreTarget, CliError> {
    let mut target = RestoreTarget::default();
    for key in SettingKey::ALL {
        let value = match state.settings.global.get(key.as_str()) {
            None => SettingState::Unset,
            Some(value) => SettingState::Value(
                SettingValue::new(value).map_err(|error| Failure::invalid(error.to_string()))?,
            ),
        };
        target.settings.push((key, value));
    }
    for (name, package_state) in &state.packages {
        let package =
            PackageName::new(name).map_err(|error| Failure::invalid(error.to_string()))?;
        target
            .packages
            .push((package, *package_state == PackageState::Enabled));
    }
    Ok(target)
}

/// Plans and applies one `history-restore` plan approved at the terminal.
fn restore(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
    text: &str,
    allow_untested: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        crate::launcher::require_terminal()?;
    }
    let (_, store) = mac_history(adb, target, which)?;
    let id = entry_id(&store, text)?;
    let state = store.state(&id).map_err(Failure::from)?;
    let serial = target.resolve(adb)?;
    let mut plan = rusttv_core::history_restore_plan(
        adb,
        &serial,
        &target_of(&state)?,
        &rusttv_core::bundled_profiles(),
        &rusttv_core::journals_dir()?,
        allow_untested,
    )?;
    let takeover_now = record::observe(adb, &serial)?.device.takeover;
    if state.device.takeover.is_some() && state.device.takeover != takeover_now {
        plan = plan.with_notes([format!(
            "The entry was recorded with Home {}; this plan doesn't change that: use `rusttv \
             launcher {}`.",
            if state.device.takeover == Some(true) {
                "taken over by RustTV"
            } else {
                "not taken over"
            },
            if state.device.takeover == Some(true) {
                "takeover"
            } else {
                "restore"
            }
        )]);
    }
    if !json {
        println!(
            "Restore entry {} ({}):\n",
            &id.as_str()[..12],
            state_summary(&store, &id)
        );
    }
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;
    let (path, journal) = apply_recorded(adb, &plan, &approval, HISTORY_RESTORE, None)?;
    report_change(&path, journal.status, json);
    if !json {
        println!(
            "Undo with `rusttv history restore --journal {}`.",
            path.display()
        );
    }
    Ok(())
}

fn state_summary(store: &Store, id: &EntryId) -> String {
    store.entry(id).map_or_else(
        |_| String::new(),
        |entry| {
            format!(
                "{}, {}",
                entry.summary,
                words::time(entry.time_unix).trim_end()
            )
        },
    )
}

fn report_change(path: &Path, status: JournalStatus, json: bool) {
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status,
        });
    } else {
        println!("\nDone.\nJournal: {}", path.display());
    }
}

fn restore_from_journal(
    adb: &dyn Adb,
    target: &Target,
    path: &Path,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        crate::launcher::require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let mut journal = rusttv_core::load_journal(path)?;
    let plan = match rusttv_core::history_restore_undo_plan(
        adb,
        &serial,
        &mut journal,
        &rusttv_core::bundled_profiles(),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            // Nothing is left to undo: record that (a file on this computer).
            if journal.status == JournalStatus::Restored && !dry_run {
                FileJournal::new(path)
                    .save(&journal)
                    .map_err(PlanError::journal)?;
                output::print_note(&format!(
                    "Marked {} restored: nothing in it is left to undo.",
                    path.display()
                ));
            }
            return Err(error.into());
        }
    };
    if !json {
        println!("Journal: {}\n", path.display());
    }
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;
    let left_as_is = restore_journal(adb, &plan, &approval, path, &mut journal)?;
    for note in &left_as_is {
        output::print_note(note);
    }
    report_change(path, journal.status, json);
    Ok(())
}

/// The history to put back on the TV: `--history`, or the only one this
/// computer holds for the TV's serial number.
fn restore_tv(
    adb: &dyn Adb,
    target: &Target,
    which: &Which,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        crate::launcher::require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    if !record::launcher_installed(adb, &serial)? {
        return Err(PlanError::precondition(
            "the RustTV launcher isn't installed; install it with `rusttv launcher install` \
             first"
                .to_owned(),
        )
        .into());
    }
    let id = match which.id()? {
        Some(id) => id,
        None => {
            let serialno = mac::serialno(adb, &serial)?.ok_or_else(|| {
                Failure::new(
                    "HISTORY_NOT_FOUND",
                    "the TV reports no serial number; pass --history <id>".to_owned(),
                )
            })?;
            let mut fits = Vec::new();
            for id in mac::Devices::load()?.histories(&serialno) {
                if let Some(store) = mac::existing(&id)?
                    && store.head().map_err(Failure::from)?.is_some()
                {
                    fits.push(id);
                }
            }
            match fits.as_slice() {
                [] => {
                    return Err(Failure::new(
                        "HISTORY_NOT_FOUND",
                        "this computer holds no history for this TV".to_owned(),
                    )
                    .into());
                }
                [id] => id.clone(),
                _ => {
                    let ids: Vec<String> = fits.iter().map(ToString::to_string).collect();
                    return Err(Failure::new(
                        "HISTORY_AMBIGUOUS",
                        format!(
                            "this computer holds several histories for this TV; pass --history \
                             with one of: {}",
                            ids.join(", ")
                        ),
                    )
                    .into());
                }
            }
        }
    };
    let store = mac::existing(&id)?.ok_or_else(|| {
        Failure::new(
            "HISTORY_NOT_FOUND",
            format!("this computer has no copy of history {id}"),
        )
    })?;
    let head = store
        .head()
        .map_err(Failure::from)?
        .ok_or_else(|| Failure::new("HISTORY_NOT_FOUND", format!("history {id} is empty")))?;

    // Only into an empty history (the TV refuses anything else too).
    let held = match door::export(adb, &serial, Some(&head))? {
        door::ExportReply::Export { .. } => Some(head.clone()),
        door::ExportReply::Refused(code) if code == "HISTORY_NOT_FOUND" => {
            match door::export(adb, &serial, None)? {
                door::ExportReply::Export { header, .. } => header.head,
                door::ExportReply::Refused(code) => {
                    return Err(Failure::refused(&code, "reading its history").into());
                }
            }
        }
        door::ExportReply::Refused(code) => {
            return Err(Failure::refused(&code, "reading its history").into());
        }
    };
    if let Some(held) = held {
        return Err(Failure::new(
            "HISTORY_NOT_EMPTY",
            format!(
                "the TV already has a history (newest entry {}); it can't take another one, and \
                 history {id} stays readable here: `rusttv history log --history {id}`",
                &held.as_str()[..12]
            ),
        )
        .into());
    }
    let export = store.export(None).map_err(Failure::from)?;
    if export.more {
        return Err(Failure::invalid(format!(
            "history {id} has more than {} entries, more than the TV takes back in one import",
            rusttv_history::EXPORT_PART_COMMITS
        ))
        .into());
    }
    let entries = store.log(usize::MAX).map_err(Failure::from)?.len();
    let mut bytes = ExportHeader {
        history: Some(id.clone()),
        head: export.head.clone(),
        more: false,
    }
    .encode()
    .into_bytes();
    bytes.extend_from_slice(&export.pack);
    let size = bytes.len();
    let head_since =
        HistorySince::entry(head.as_str()).map_err(|error| Failure::invalid(error.to_string()))?;
    let plan = rusttv_core::restore_tv_plan(
        adb,
        &serial,
        bytes,
        &head_since,
        door::nonce(),
        vec![
            format!(
                "Puts history {id} back on the TV: {entries} entr{}, newest {}, {} KB, from this \
                 computer's copy.",
                if entries == 1 { "y" } else { "ies" },
                &head.as_str()[..12],
                size.div_ceil(1024)
            ),
            "Launcher-private (RL): both steps only write RustTV's own history on the TV; no \
             setting or app changes. The TV takes it only into an empty history, labels every \
             entry \"Restored from your computer\", and records its state now as a new entry. \
             Pending approvals and journals aren't part of it."
                .to_owned(),
            "It stays on the TV (a history is never rewritten); this computer keeps its copy."
                .to_owned(),
        ],
    )?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;
    let (path, journal) = apply_recorded(adb, &plan, &approval, HISTORY_RESTORE_TV, None)?;
    report_change(&path, journal.status, json);
    Ok(())
}

/// `history allow-tv-settings` / `revoke-tv-settings`.
fn tv_settings(
    adb: &dyn Adb,
    target: &Target,
    grant: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        crate::launcher::require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::tv_settings_plan(adb, &serial, grant)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;
    let name = plan.name().to_owned();
    let (path, journal) = apply_recorded(adb, &plan, &approval, &name, None)?;
    report_change(&path, journal.status, json);
    Ok(())
}

/// One applied pending approval in `--json` output.
#[derive(Debug, Serialize)]
struct PendingReport {
    /// The approval's SHA-256.
    approval: String,
    /// The plan was exactly what was approved on the TV (else a person
    /// typed the confirmation here).
    approved_on_tv: bool,
    journal: String,
    status: JournalStatus,
}

/// `history apply-pending` (HISTORY.md § 6.3).
fn apply_pending(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    crate::launcher::require_terminal()?;
    let serial = target.resolve(adb)?;
    let info = adb.run(
        &serial,
        &rusttv_core::DeviceCommand::PackageInfo(record::launcher_package()?),
    )?;
    match rusttv_core::launcher_test_build(&info) {
        None if rusttv_core::launcher_grant(&info).is_none() => {
            return Err(
                PlanError::precondition("the RustTV launcher isn't installed".to_owned()).into(),
            );
        }
        Some(false) => {}
        _ => {
            return Err(Failure::new(
                "PENDING_REFUSED",
                "the installed launcher is a test build (debuggable or profileable), whose \
                 approvals aren't trusted; nothing was applied"
                    .to_owned(),
            )
            .into());
        }
    }
    let list = door::pending(adb, &serial)?;
    if list.approval.is_empty() {
        if json {
            output::print_json(&Vec::<PendingReport>::new());
        } else {
            println!("No approval made on the TV is waiting.");
        }
        return Ok(());
    }
    let mut reports = Vec::new();
    for approval in &list.approval {
        reports.push(apply_one(adb, &serial, approval, json)?);
    }
    if json {
        output::print_json(&reports);
    }
    Ok(())
}

fn apply_one(
    adb: &dyn Adb,
    serial: &str,
    approval: &PendingApproval,
    json: bool,
) -> Result<PendingReport, CliError> {
    let changes: Vec<TvChange> = approval
        .change
        .iter()
        .map(tv_change)
        .collect::<Result<_, _>>()?;
    // Claimed first: the TV marks it used once, so it can't be applied twice.
    door::send_message(
        adb,
        serial,
        &Message::Consume {
            approval: approval.hash.clone(),
        },
        "the approval",
    )?;
    let mut target = RestoreTarget::default();
    for change in &changes {
        match change {
            TvChange::Setting { key, new, .. } => target.settings.push((*key, new.clone())),
            TvChange::Package { package, new, .. } => target.packages.push((package.clone(), *new)),
        }
    }
    let plan = rusttv_core::history_restore_plan(
        adb,
        serial,
        &target,
        &rusttv_core::bundled_profiles(),
        &rusttv_core::journals_dir()?,
        false,
    )?;
    let by_tv = rusttv_core::approve_by_tv(&plan, &approval.hash, &changes);
    let approved_on_tv = by_tv.is_ok();
    let (approval_done, plan) = match by_tv {
        Ok(done) => {
            show_plan(&plan, false, json);
            output::print_note("Approved on the TV: this is exactly what was approved there.");
            (done, plan)
        }
        Err(why) => {
            output::print_note(&format!(
                "{}\nApproved on the TV:\n{}\n\nThe plan now:",
                output::message_with_causes(&why),
                approval
                    .change
                    .iter()
                    .map(|change| format!("  {}", approved_text(change)))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            show_plan(&plan, false, json);
            let typed = read_confirmation(plan.confirmation(), "apply this plan instead")?;
            (plan.approve(&typed)?, plan)
        }
    };
    let (path, journal) = apply_recorded(
        adb,
        &plan,
        &approval_done,
        HISTORY_RESTORE,
        Some(&approval.hash),
    )?;
    if !json {
        println!("\nDone.\nJournal: {}", path.display());
    }
    Ok(PendingReport {
        approval: approval.hash.clone(),
        approved_on_tv,
        journal: path.display().to_string(),
        status: journal.status,
    })
}

fn tv_change(change: &ApprovedChange) -> Result<TvChange, CliError> {
    let invalid = |what: String| -> CliError { Failure::invalid(what).into() };
    let state = |value: &Option<String>| match value {
        None => Ok(SettingState::Unset),
        Some(value) => SettingValue::new(value)
            .map(SettingState::Value)
            .map_err(|error| invalid(error.to_string())),
    };
    Ok(match change {
        ApprovedChange::Setting { key, old, new } => TvChange::Setting {
            key: SettingKey::ALL
                .into_iter()
                .find(|known| known.as_str() == key)
                .ok_or_else(|| invalid(format!("unknown setting {key}")))?,
            old: state(old)?,
            new: state(new)?,
        },
        ApprovedChange::Package { name, old, new } => TvChange::Package {
            package: PackageName::new(name).map_err(|error| invalid(error.to_string()))?,
            old: old.map(|old| old == PackageState::Enabled),
            new: *new == PackageState::Enabled,
        },
    })
}

/// `launcher uninstall` (and any plan that reinstalls the launcher) first
/// syncs, so this computer's copy is complete (HISTORY.md § 4.5). If that
/// fails, a person at the terminal may go on anyway by typing `continue`.
pub(crate) fn sync_before_removal(adb: &dyn Adb, serial: &str) -> Result<(), CliError> {
    if !record::launcher_installed(adb, serial)? {
        return Ok(());
    }
    let error = match sync::sync(adb, serial, None) {
        Ok(synced) => {
            if let Some(id) = &synced.history {
                output::print_note(&format!(
                    "Synced the TV's history {id} first: this computer's copy is complete."
                ));
            }
            return Ok(());
        }
        Err(error) => error,
    };
    output::print_note(&format!(
        "The TV's history couldn't be copied to this computer first: {}. Uninstalling deletes \
         it on the TV; what this computer doesn't have is lost.",
        output::message_with_causes(&error)
    ));
    if !rusttv_core::stdin_is_terminal() {
        return Err(error);
    }
    let typed = read_confirmation("continue", "uninstall anyway")?;
    if typed.trim() == "continue" {
        Ok(())
    } else {
        Err(PlanError::not_approved("`continue` was not typed; nothing changed").into())
    }
}
