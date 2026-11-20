//! Panels: RustTV features that show themselves on several faces (PANELS.md).
//!
//! A panel is a pure function from data Home already has to a small view.
//! The view holds values for each face's template properties by name
//! (`templates/*.json` `props`); RustTV's generated templates draw them,
//! and a page's layout and blocks live in its template, not here.
//! Views carry kinds and values, never translated words: the template
//! translates (`@tr`). No trait until a second panel exists (PANELS.md § 3).

use crate::platform::UpdateStatus;

/// One value per template property, in the template's `props` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Props(pub Vec<(&'static str, String)>);

impl Props {
    /// The value of `name`, or "" for a property the view doesn't set.
    pub fn get(&self, name: &str) -> &str {
        self.0
            .iter()
            .find(|(prop, _)| *prop == name)
            .map_or("", |(_, value)| value.as_str())
    }
}

/// What a panel's tile shows (a `tile` template).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelTile {
    pub props: Props,
}

/// What a panel's row in a list shows (a `row` template).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelRow {
    pub props: Props,
}

/// What a panel's page shows (a `page` template). Home owns its focus and
/// keys, and the long text of its scroll box (the template names which).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelPage {
    pub props: Props,
}

/// Which faces a panel has: `None` for a face the panel doesn't have. About
/// has all three, and Home relies on it (tests check it): the About page and
/// Settings' RustTV row are always there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelView {
    pub tile: Option<PanelTile>,
    pub row: Option<PanelRow>,
    pub page: Option<PanelPage>,
}

/// About (templates/about_*.json). The tile shows the version, and a caption
/// while an update is available, downloading or installing; anything else, a
/// failure included, shows no caption (the reason is on the page). The row
/// shows the version. The page shows the version and the updater's state
/// with what it names (a version, a percent or a reason).
pub(crate) fn about_view(version: &str, status: &UpdateStatus) -> PanelView {
    let caption = match status {
        UpdateStatus::Available(_) | UpdateStatus::NeedsPermission(_) => "update",
        UpdateStatus::Downloading(_) => "downloading",
        UpdateStatus::Installing => "installing",
        UpdateStatus::Idle
        | UpdateStatus::Checking
        | UpdateStatus::UpToDate
        | UpdateStatus::Installed
        | UpdateStatus::Failed(_) => "",
    };
    let (update, detail) = match status {
        UpdateStatus::Idle => ("idle", String::new()),
        UpdateStatus::Checking => ("checking", String::new()),
        UpdateStatus::UpToDate => ("up-to-date", String::new()),
        UpdateStatus::Available(version) => ("available", version.clone()),
        UpdateStatus::NeedsPermission(version) => ("needs-permission", version.clone()),
        UpdateStatus::Downloading(percent) => ("downloading", percent.to_string()),
        UpdateStatus::Installing => ("installing", String::new()),
        UpdateStatus::Installed => ("installed", String::new()),
        UpdateStatus::Failed(reason) => ("failed", reason.clone()),
    };
    PanelView {
        tile: Some(PanelTile {
            props: Props(vec![
                ("version", version.to_owned()),
                ("status", caption.to_owned()),
            ]),
        }),
        row: Some(PanelRow {
            props: Props(vec![("version", version.to_owned())]),
        }),
        page: Some(PanelPage {
            props: Props(vec![
                ("version", version.to_owned()),
                ("update", update.to_owned()),
                ("update-detail", detail),
            ]),
        }),
    }
}

/// Every `UpdateStatus` with a detail, for tests across modules.
#[cfg(test)]
pub(crate) const UPDATE_STATUSES: [UpdateStatus; 9] = [
    UpdateStatus::Idle,
    UpdateStatus::Checking,
    UpdateStatus::UpToDate,
    UpdateStatus::Available(String::new()),
    UpdateStatus::NeedsPermission(String::new()),
    UpdateStatus::Downloading(40),
    UpdateStatus::Installing,
    UpdateStatus::Installed,
    UpdateStatus::Failed(String::new()),
];

/// About's page template's updater line: the text that selects on `update`.
#[cfg(test)]
pub(crate) fn updater_line(page: &serde_json::Value) -> &serde_json::Value {
    ["start", "end"]
        .iter()
        .flat_map(|side| page["root"]["page"][side].as_array().expect("blocks"))
        .filter_map(|block| block.get("paragraph"))
        .find(|text| text["value"]["select"] == "update")
        .expect("the updater's line")
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::UPDATE_STATUSES as STATUSES;

    fn caption(status: &UpdateStatus) -> String {
        about_view("1.2.3", status)
            .tile
            .expect("About always has a tile")
            .props
            .get("status")
            .to_owned()
    }

    #[test]
    fn about_caption_follows_every_update_status() {
        let cases = [
            (UpdateStatus::Idle, ""),
            (UpdateStatus::Checking, ""),
            (UpdateStatus::UpToDate, ""),
            (UpdateStatus::Available("2.0.0".into()), "update"),
            (UpdateStatus::NeedsPermission("2.0.0".into()), "update"),
            // No percent, so the download's progress doesn't redraw Home.
            (UpdateStatus::Downloading(10), "downloading"),
            (UpdateStatus::Downloading(90), "downloading"),
            (UpdateStatus::Installing, "installing"),
            (UpdateStatus::Installed, ""),
            (UpdateStatus::Failed("no network".into()), ""),
        ];
        for (status, expected) in cases {
            assert_eq!(caption(&status), expected, "{status:?}");
        }
    }

    #[test]
    fn about_page_names_every_update_status_and_its_detail() {
        let cases = [
            (UpdateStatus::Idle, "idle", ""),
            (UpdateStatus::Checking, "checking", ""),
            (UpdateStatus::UpToDate, "up-to-date", ""),
            (
                UpdateStatus::Available("2.0.0".into()),
                "available",
                "2.0.0",
            ),
            (
                UpdateStatus::NeedsPermission("2.0.0".into()),
                "needs-permission",
                "2.0.0",
            ),
            (UpdateStatus::Downloading(40), "downloading", "40"),
            (UpdateStatus::Installing, "installing", ""),
            (UpdateStatus::Installed, "installed", ""),
            (
                UpdateStatus::Failed("no network".into()),
                "failed",
                "no network",
            ),
        ];
        for (status, update, detail) in cases {
            let page = about_view("1.2.3", &status).page.expect("About has a page");
            assert_eq!(page.props.get("update"), update, "{status:?}");
            assert_eq!(page.props.get("update-detail"), detail, "{status:?}");
        }
    }

    #[test]
    fn about_faces_carry_the_version() {
        let view = about_view("1.2.3", &UpdateStatus::Idle);
        assert_eq!(view.tile.expect("tile").props.get("version"), "1.2.3");
        assert_eq!(view.row.expect("row").props.get("version"), "1.2.3");
        assert_eq!(view.page.expect("page").props.get("version"), "1.2.3");
    }

    fn template(name: &str) -> serde_json::Value {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let json =
            std::fs::read_to_string(dir.join(format!("templates/{name}.json"))).expect("template");
        serde_json::from_str(&json).expect("JSON")
    }

    fn names(props: &Props) -> Vec<&str> {
        props.0.iter().map(|(name, _)| *name).collect()
    }

    /// Every property each face's view sets is one its template declares, and
    /// the other way round, so nothing is dropped or left empty by mistake;
    /// the page's status line has a text for every state the view names
    /// ("idle" is its `else`), and no other.
    #[test]
    fn about_view_matches_its_templates() {
        let page = template("about_page");
        let status = updater_line(&page);
        let cases: Vec<&str> = status["value"]["cases"]
            .as_object()
            .expect("cases")
            .keys()
            .map(String::as_str)
            .collect();
        let mut named: Vec<String> = STATUSES
            .iter()
            .filter_map(|status| about_view("1.2.3", status).page)
            .map(|page| page.props.get("update").to_owned())
            .filter(|update| update != "idle")
            .collect();
        named.sort_unstable();
        named.dedup();
        let mut cases = cases;
        cases.sort_unstable();
        assert_eq!(cases, named);

        let declared = |name: &str| -> Vec<String> {
            template(name)["props"]
                .as_array()
                .expect("props")
                .iter()
                .map(|prop| prop.as_str().expect("string").to_owned())
                .collect()
        };
        let (tile, row, page) = (
            declared("about_tile"),
            declared("about_row"),
            declared("about_page"),
        );
        for status in &STATUSES {
            let view = about_view("1.2.3", status);
            let faces = [
                (&tile, view.tile.map(|tile| tile.props)),
                (&row, view.row.map(|row| row.props)),
                (&page, view.page.map(|page| page.props)),
            ];
            for (declared, props) in faces {
                let props = props.expect("About has every face");
                assert_eq!(names(&props), *declared, "{status:?}");
            }
        }
    }

    /// [`UPDATE_STATUSES`] holds each variant once: a new variant fails to
    /// compile here until it is given a place, and then the list must hold it.
    #[test]
    fn statuses_list_every_variant() {
        let place = |status: &UpdateStatus| match status {
            UpdateStatus::Idle => 0,
            UpdateStatus::Checking => 1,
            UpdateStatus::UpToDate => 2,
            UpdateStatus::Available(_) => 3,
            UpdateStatus::NeedsPermission(_) => 4,
            UpdateStatus::Downloading(_) => 5,
            UpdateStatus::Installing => 6,
            UpdateStatus::Installed => 7,
            UpdateStatus::Failed(_) => 8,
        };
        let places: Vec<usize> = STATUSES.iter().map(place).collect();
        assert_eq!(places, (0..STATUSES.len()).collect::<Vec<_>>());
    }

    /// The About page keeps Slint's attribution (its Royalty-free License)
    /// and scrolls the licence notices, which Home loads when it opens.
    #[test]
    fn about_page_keeps_the_attributions() {
        let page = template("about_page");
        let blocks: Vec<&serde_json::Value> = ["start", "end"]
            .iter()
            .flat_map(|side| page["root"]["page"][side].as_array().expect("blocks"))
            .collect();
        assert!(blocks.iter().any(|block| block["mark"]["name"] == "slint"));
        assert!(
            blocks
                .iter()
                .any(|block| block["scroll-box"]["source"] == "notices")
        );
    }

    /// Every text a template shows, as `@tr` message ids.
    fn texts(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(text)) = map.get("tr") {
                    out.push(text.clone());
                }
                map.values().for_each(|v| texts(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| texts(v, out)),
            _ => {}
        }
    }

    /// A gettext file's entries as (msgid, msgstr), still escaped, with
    /// wrapped lines (`msgid ""` then `"…"` lines) joined.
    fn entries(file: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut last = None;
        for line in file.lines() {
            let (key, rest) = if let Some(rest) = line.strip_prefix("msgid ") {
                out.push((String::new(), String::new()));
                (Some(0), rest)
            } else if let Some(rest) = line.strip_prefix("msgstr ") {
                (Some(1), rest)
            } else if line.starts_with('"') {
                (last, line)
            } else {
                last = None;
                continue;
            };
            last = key;
            let text = rest
                .trim()
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .unwrap_or_default();
            if let (Some(field), Some(entry)) = (key, out.last_mut()) {
                if field == 0 {
                    &mut entry.0
                } else {
                    &mut entry.1
                }
                .push_str(text);
            }
        }
        out
    }

    /// Template texts reach translators (scripts/launcher-pot.sh) and every
    /// language translates them: generated Slint lives in OUT_DIR, where a
    /// plain extractor run over ui/ would miss it.
    #[test]
    fn template_texts_are_translated_everywhere() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(dir.join("templates")).expect("templates/") {
            let path = entry.expect("entry").path();
            let json = std::fs::read_to_string(&path).expect("template");
            texts(&serde_json::from_str(&json).expect("JSON"), &mut ids);
        }
        assert!(!ids.is_empty());
        // Template texts hold no quotes or backslashes (convert.rs would
        // escape them), so their escaped form is the text itself.
        assert!(ids.iter().all(|id| !id.contains(['"', '\\'])));
        let pot =
            std::fs::read_to_string(dir.join("translations/rusttv-launcher.pot")).expect("pot");
        let pot = entries(&pot);
        for id in &ids {
            assert!(
                pot.iter().any(|(msgid, _)| msgid == id),
                "{id:?} missing from the .pot"
            );
        }
        for language in crate::i18n::LANGUAGES {
            let po = dir.join(format!(
                "translations/{language}/LC_MESSAGES/rusttv-launcher.po"
            ));
            let po = entries(&std::fs::read_to_string(po).expect("po"));
            for id in &ids {
                let (_, msgstr) = po
                    .iter()
                    .find(|(msgid, _)| msgid == id)
                    .unwrap_or_else(|| panic!("{language}: {id:?} missing"));
                assert!(!msgstr.is_empty(), "{language}: {id:?} is not translated");
            }
        }
    }
}
