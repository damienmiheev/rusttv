//! Panels: RustTV features that show themselves on several faces (PANELS.md).
//!
//! A panel is a pure function from data Home already has to a small view.
//! The view holds values for the template's properties by name
//! (`templates/*.json` `props`); RustTV's generated templates draw them.
//! Views carry kinds and values, never translated words: the template
//! translates (`@tr`). No trait until a second panel exists (PANELS.md § 3).

use crate::platform::UpdateStatus;

/// What a panel's tile shows: one value per template property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelTile {
    /// `(property, value)` in the template's `props` order.
    pub props: Vec<(&'static str, String)>,
}

impl PanelTile {
    /// The value of `name`, or "" for a property the view doesn't set.
    pub fn get(&self, name: &str) -> &str {
        self.props
            .iter()
            .find(|(prop, _)| *prop == name)
            .map_or("", |(_, value)| value.as_str())
    }
}

/// Which faces a panel has right now: a face is shown only when it is `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelView {
    pub tile: Option<PanelTile>,
}

/// About (templates/about_tile.json): the version, and a caption while an
/// update is available, downloading or installing. Anything else, a failure
/// included, shows no caption; the reason is on the About page only.
pub(crate) fn about_view(version: &str, status: &UpdateStatus) -> PanelView {
    let status = match status {
        UpdateStatus::Available(_) | UpdateStatus::NeedsPermission(_) => "update",
        UpdateStatus::Downloading(_) => "downloading",
        UpdateStatus::Installing => "installing",
        UpdateStatus::Idle
        | UpdateStatus::Checking
        | UpdateStatus::UpToDate
        | UpdateStatus::Installed
        | UpdateStatus::Failed(_) => "",
    };
    PanelView {
        tile: Some(PanelTile {
            props: vec![
                ("version", version.to_owned()),
                ("status", status.to_owned()),
            ],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caption(status: &UpdateStatus) -> String {
        about_view("1.2.3", status)
            .tile
            .expect("About always has a tile")
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
    fn about_tile_carries_the_version() {
        let view = about_view("1.2.3", &UpdateStatus::Idle);
        assert_eq!(view.tile.expect("tile").get("version"), "1.2.3");
    }

    /// Every property the view sets is one the template declares, and the
    /// other way round, so nothing is dropped or left empty by mistake.
    #[test]
    fn about_view_matches_its_template() {
        let template: serde_json::Value =
            serde_json::from_str(include_str!("../templates/about_tile.json")).expect("JSON");
        let declared: Vec<&str> = template["props"]
            .as_array()
            .expect("props")
            .iter()
            .map(|prop| prop.as_str().expect("string"))
            .collect();
        let tile = about_view("1.2.3", &UpdateStatus::Idle).tile.expect("tile");
        let set: Vec<&str> = tile.props.iter().map(|(name, _)| *name).collect();
        assert_eq!(set, declared);
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
        let pot =
            std::fs::read_to_string(dir.join("translations/rusttv-launcher.pot")).expect("pot");
        for id in &ids {
            assert!(
                pot.contains(&format!("msgid \"{id}\"\n")),
                "{id:?} missing from the .pot"
            );
        }
        for language in crate::i18n::LANGUAGES {
            let po = dir.join(format!(
                "translations/{language}/LC_MESSAGES/rusttv-launcher.po"
            ));
            let po = std::fs::read_to_string(po).expect("po");
            for id in &ids {
                let entry = format!("msgid \"{id}\"\nmsgstr \"");
                let at = po
                    .find(&entry)
                    .unwrap_or_else(|| panic!("{language}: {id:?} missing"));
                assert!(
                    !po[at + entry.len()..].starts_with('"'),
                    "{language}: {id:?} is not translated"
                );
            }
        }
    }
}
