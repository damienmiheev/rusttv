//! What changed between two snapshots, as data: the TV and the CLI turn it
//! into words in their own language (HISTORY.md § 2).

use std::collections::BTreeSet;

use crate::state::{PackageState, SETTING_KEYS, State};

/// One difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// The tile order changed (tiles that moved, in their new order).
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

    let common: Vec<&String> = new
        .launcher
        .order
        .iter()
        .filter(|tile| old.launcher.order.contains(tile))
        .collect();
    let before: Vec<&String> = old
        .launcher
        .order
        .iter()
        .filter(|tile| new.launcher.order.contains(tile))
        .collect();
    if common != before {
        let moved = common
            .iter()
            .zip(&before)
            .filter(|(now, was)| now != was)
            .map(|(now, _)| (*now).clone())
            .collect();
        changes.push(Change::Reordered { moved });
    }
    for tile in &new.launcher.order {
        if !old.launcher.order.contains(tile) {
            changes.push(Change::Added { tile: tile.clone() });
        }
    }
    for tile in &old.launcher.order {
        if !new.launcher.order.contains(tile) {
            changes.push(Change::Removed { tile: tile.clone() });
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
