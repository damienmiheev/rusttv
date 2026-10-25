//! What changed between two snapshots, as data: the TV and the CLI turn it
//! into words in their own language (HISTORY.md § 2).

use std::collections::BTreeSet;

use crate::state::{Launcher, PackageState, SETTING_KEYS, State};

/// One difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// The tile order changed, on Home or in a folder (tiles that moved,
    /// in their new order).
    Reordered {
        /// Tiles whose position changed.
        moved: Vec<String>,
    },
    /// A tile appeared on Home (an app was installed, or a built-in tile
    /// came with an update).
    Added {
        /// The tile's name.
        tile: String,
    },
    /// A tile left Home (an app was removed).
    Removed {
        /// The tile's name.
        tile: String,
    },
    /// A folder was made.
    FolderAdded {
        /// Its tile name (`folder:<number>`).
        folder: String,
        /// Its name.
        name: String,
    },
    /// A folder went (its last app left it).
    FolderRemoved {
        /// Its tile name.
        folder: String,
        /// Its name.
        name: String,
    },
    /// A folder got a new name.
    FolderRenamed {
        /// Its tile name.
        folder: String,
        /// Before.
        old: String,
        /// After.
        new: String,
    },
    /// An app on Home went into a folder or came out of one.
    InFolder {
        /// The app's component name.
        tile: String,
        /// The folder's tile name.
        folder: String,
        /// The folder's name.
        name: String,
        /// Whether the app is in the folder now.
        inside: bool,
    },
    /// An app was hidden or shown again.
    Hidden {
        /// The app's component name.
        tile: String,
        /// Whether it is hidden now.
        hidden: bool,
    },
    /// The update switch changed.
    UpdateCheck {
        /// Its new value.
        on: bool,
    },
    /// A global setting changed; `None` = unset.
    Setting {
        /// The key.
        key: String,
        /// Before.
        old: Option<String>,
        /// After.
        new: Option<String>,
    },
    /// A package's state changed, or it appeared or disappeared from what the
    /// Mac observed; `None` = not observed.
    Package {
        /// The package name.
        name: String,
        /// Before.
        old: Option<PackageState>,
        /// After.
        new: Option<PackageState>,
    },
    /// The firmware fingerprint changed.
    Firmware {
        /// Before.
        old: Option<String>,
        /// After.
        new: Option<String>,
    },
    /// A Home takeover started or ended.
    Takeover {
        /// Whether it is active now.
        active: bool,
    },
}

/// What changed from `old` to `new`, in a fixed order: launcher, settings,
/// packages, device. Animation scales compare as numbers, unset = "1".
pub fn diff(old: &State, new: &State) -> Vec<Change> {
    let mut changes = Vec::new();
    let (was, now) = (&old.launcher, &new.launcher);

    let mut moved = moved_tiles(&was.order, &now.order);
    for (folder, then) in &now.folders {
        if let Some(before) = was.folders.get(folder) {
            moved.extend(moved_tiles(&before.apps, &then.apps));
        }
    }
    if !moved.is_empty() {
        changes.push(Change::Reordered { moved });
    }
    // Apps and built-in tiles on Home, in folders or not; folders aren't tiles here.
    let on_home = |launcher: &Launcher| -> Vec<String> {
        launcher
            .order
            .iter()
            .filter(|tile| !launcher.folders.contains_key(*tile))
            .chain(launcher.folders.values().flat_map(|folder| &folder.apps))
            .cloned()
            .collect()
    };
    let (before, after) = (on_home(was), on_home(now));
    for tile in after.iter().filter(|tile| !before.contains(tile)) {
        changes.push(Change::Added { tile: tile.clone() });
    }
    for tile in before.iter().filter(|tile| !after.contains(tile)) {
        changes.push(Change::Removed { tile: tile.clone() });
    }
    for (folder, then) in &now.folders {
        if !was.folders.contains_key(folder) {
            changes.push(Change::FolderAdded {
                folder: folder.clone(),
                name: then.name.clone(),
            });
        }
    }
    let folder_of = |launcher: &Launcher, tile: &String| {
        launcher
            .folders
            .iter()
            .find(|(_, folder)| folder.apps.contains(tile))
            .map(|(id, folder)| (id.clone(), folder.name.clone()))
    };
    for tile in after.iter().filter(|tile| before.contains(tile)) {
        let (from, to) = (folder_of(was, tile), folder_of(now, tile));
        if from.as_ref().map(|(id, _)| id) == to.as_ref().map(|(id, _)| id) {
            continue;
        }
        if let Some((folder, name)) = from {
            changes.push(Change::InFolder {
                tile: tile.clone(),
                folder,
                name,
                inside: false,
            });
        }
        if let Some((folder, name)) = to {
            changes.push(Change::InFolder {
                tile: tile.clone(),
                folder,
                name,
                inside: true,
            });
        }
    }
    for (folder, then) in &now.folders {
        if let Some(before) = was.folders.get(folder)
            && before.name != then.name
        {
            changes.push(Change::FolderRenamed {
                folder: folder.clone(),
                old: before.name.clone(),
                new: then.name.clone(),
            });
        }
    }
    for (folder, before) in &was.folders {
        if !now.folders.contains_key(folder) {
            changes.push(Change::FolderRemoved {
                folder: folder.clone(),
                name: before.name.clone(),
            });
        }
    }
    let hidden: BTreeSet<&String> = old
        .launcher
        .hidden
        .symmetric_difference(&new.launcher.hidden)
        .collect();
    for tile in hidden {
        changes.push(Change::Hidden {
            tile: tile.clone(),
            hidden: new.launcher.hidden.contains(tile),
        });
    }
    if old.launcher.update_check != new.launcher.update_check {
        changes.push(Change::UpdateCheck {
            on: new.launcher.update_check,
        });
    }

    for key in SETTING_KEYS {
        if old.setting(key) != new.setting(key) {
            changes.push(Change::Setting {
                key: key.to_owned(),
                old: old.settings.global.get(key).cloned(),
                new: new.settings.global.get(key).cloned(),
            });
        }
    }

    let names: BTreeSet<&String> = old.packages.keys().chain(new.packages.keys()).collect();
    for name in names {
        let (was, now) = (old.packages.get(name), new.packages.get(name));
        if was != now {
            changes.push(Change::Package {
                name: name.clone(),
                old: was.copied(),
                new: now.copied(),
            });
        }
    }

    if old.device.fingerprint != new.device.fingerprint {
        changes.push(Change::Firmware {
            old: old.device.fingerprint.clone(),
            new: new.device.fingerprint.clone(),
        });
    }
    if old.device.takeover != new.device.takeover {
        changes.push(Change::Takeover {
            active: new.device.takeover == Some(true),
        });
    }
    changes
}

/// Tiles in both lists whose place among the tiles they share changed, in
/// their new order.
fn moved_tiles(old: &[String], new: &[String]) -> Vec<String> {
    let common = new.iter().filter(|tile| old.contains(tile));
    let before = old.iter().filter(|tile| new.contains(tile));
    common
        .zip(before)
        .filter(|(now, was)| now != was)
        .map(|(now, _)| now.clone())
        .collect()
}

/// A one-line English summary, for `git log` on the Mac (HISTORY.md § 2).
/// People see words made from the [`Change`]s in their own language instead.
pub(crate) fn summary(changes: &[Change]) -> String {
    let Some(first) = changes.first() else {
        return "No change".to_owned();
    };
    let one = match first {
        Change::Reordered { .. } => "Reordered apps".to_owned(),
        Change::Added { tile } => format!("Added {}", tile.split('/').next().unwrap_or(tile)),
        Change::Removed { tile } => format!("Removed {}", tile.split('/').next().unwrap_or(tile)),
        Change::Hidden { tile, hidden } => {
            let app = tile.split('/').next().unwrap_or(tile);
            format!("{} {app}", if *hidden { "Hid" } else { "Showed" })
        }
        Change::FolderAdded { name, .. } => format!("Made folder {name}"),
        Change::FolderRemoved { name, .. } => format!("Removed folder {name}"),
        Change::FolderRenamed { old, new, .. } => format!("Renamed folder {old} to {new}"),
        Change::InFolder {
            tile, name, inside, ..
        } => {
            let app = tile.split('/').next().unwrap_or(tile);
            if *inside {
                format!("Put {app} in {name}")
            } else {
                format!("Took {app} out of {name}")
            }
        }
        Change::UpdateCheck { on } => {
            format!("Update check {}", if *on { "on" } else { "off" })
        }
        Change::Setting { key, new, .. } => {
            format!("{key} = {}", new.as_deref().unwrap_or("unset"))
        }
        Change::Package { name, new, .. } => match new {
            Some(state) => format!("{name} {state}"),
            None => format!("{name} not seen"),
        },
        Change::Firmware { .. } => "Firmware changed".to_owned(),
        Change::Takeover { active } => {
            format!("Home takeover {}", if *active { "on" } else { "off" })
        }
    };
    match changes.len() {
        1 => one,
        n => format!("{one} and {} more", n - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_changed_is_empty() {
        assert!(diff(&State::new(), &State::new()).is_empty());
    }

    #[test]
    fn reports_each_kind_of_change() {
        let old = State::new();
        let mut new = State::new();
        new.launcher.hidden.insert("com.example.a/.Main".into());
        new.launcher.update_check = false;
        new.settings
            .global
            .insert("animator_duration_scale".into(), "0.5".into());
        new.packages
            .insert("com.example.t".into(), PackageState::Disabled);
        new.device.takeover = Some(true);
        let changes = diff(&old, &new);
        assert_eq!(changes.len(), 5, "{changes:?}");
        assert_eq!(summary(&changes), "Hid com.example.a and 4 more");
    }

    #[test]
    fn a_move_names_the_tiles_that_moved_and_new_apps_are_added() {
        let mut old = State::new();
        old.launcher.order = ["a.b/.A", "c.d/.C", "about"].map(String::from).to_vec();
        let mut new = State::new();
        new.launcher.order = ["c.d/.C", "a.b/.A", "e.f/.New", "about"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            diff(&old, &new),
            vec![
                Change::Reordered {
                    moved: vec!["c.d/.C".into(), "a.b/.A".into()]
                },
                Change::Added {
                    tile: "e.f/.New".into()
                }
            ]
        );
        let mut added = old.clone();
        added.launcher.order.push("e.f/.New".into());
        assert_eq!(
            diff(&old, &added),
            vec![Change::Added {
                tile: "e.f/.New".into()
            }]
        );
        assert_eq!(
            diff(&added, &old),
            vec![Change::Removed {
                tile: "e.f/.New".into()
            }]
        );
    }

    fn folder(name: &str, apps: &[&str]) -> crate::state::Folder {
        crate::state::Folder {
            name: name.into(),
            apps: apps.iter().map(|app| (*app).to_owned()).collect(),
        }
    }

    #[test]
    fn making_a_folder_puts_apps_in_it_without_adding_or_removing_them() {
        let mut old = State::new();
        old.launcher.order = ["a.b/.A", "c.d/.C", "e.f/.E", "about"]
            .map(String::from)
            .to_vec();
        let mut new = State::new();
        new.launcher.order = ["folder:1", "e.f/.E", "about"].map(String::from).to_vec();
        new.launcher
            .folders
            .insert("folder:1".into(), folder("Games", &["c.d/.C", "a.b/.A"]));
        let changes = diff(&old, &new);
        let inside = |tile: &str| Change::InFolder {
            tile: tile.into(),
            folder: "folder:1".into(),
            name: "Games".into(),
            inside: true,
        };
        assert_eq!(
            changes,
            vec![
                Change::FolderAdded {
                    folder: "folder:1".into(),
                    name: "Games".into()
                },
                inside("c.d/.C"),
                inside("a.b/.A"),
            ]
        );
        assert_eq!(summary(&changes), "Made folder Games and 2 more");

        // Back out: the folder goes, the apps come out.
        let back = diff(&new, &old);
        assert_eq!(back.len(), 3, "{back:?}");
        assert!(back.contains(&Change::FolderRemoved {
            folder: "folder:1".into(),
            name: "Games".into()
        }));
    }

    #[test]
    fn moving_between_folders_renaming_and_reordering_inside_are_reported() {
        let mut old = State::new();
        old.launcher.order = ["folder:1", "folder:2"].map(String::from).to_vec();
        old.launcher.folders.insert(
            "folder:1".into(),
            folder("A", &["a.b/.A", "c.d/.C", "x.y/.X"]),
        );
        old.launcher
            .folders
            .insert("folder:2".into(), folder("B", &["e.f/.E"]));
        let mut new = old.clone();
        new.launcher
            .folders
            .insert("folder:1".into(), folder("Films", &["c.d/.C", "a.b/.A"]));
        new.launcher
            .folders
            .insert("folder:2".into(), folder("B", &["e.f/.E", "x.y/.X"]));
        assert_eq!(
            diff(&old, &new),
            vec![
                Change::Reordered {
                    moved: vec!["c.d/.C".into(), "a.b/.A".into()]
                },
                Change::InFolder {
                    tile: "x.y/.X".into(),
                    folder: "folder:1".into(),
                    name: "A".into(),
                    inside: false
                },
                Change::InFolder {
                    tile: "x.y/.X".into(),
                    folder: "folder:2".into(),
                    name: "B".into(),
                    inside: true
                },
                Change::FolderRenamed {
                    folder: "folder:1".into(),
                    old: "A".into(),
                    new: "Films".into()
                },
            ]
        );
    }

    #[test]
    fn one_and_unset_are_the_same_scale() {
        let old = State::new();
        let mut new = State::new();
        new.settings
            .global
            .insert("window_animation_scale".into(), "1.0".into());
        assert!(diff(&old, &new).is_empty());
    }
}
