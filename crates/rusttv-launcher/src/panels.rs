//! Panels: RustTV features that show themselves on several faces (PANELS.md).
//!
//! A panel is a pure function from data Home already has to a small view.
//! The view holds values for each face's template properties by name
//! (`templates/*.json` `props`); RustTV's generated templates draw them,
//! and a page's layout and blocks live in its template, not here.
//! Views carry kinds and values, never translated words: the template
//! translates (`@tr`). No trait: About's, Remote's and History's views take
//! different data and share only [`PanelView`] (PANELS.md § 3).

use crate::history::Summary;
use crate::platform::UpdateStatus;

/// Times before this (2024-01-01, Unix seconds) mean the TV's clock wasn't
/// set: no age is shown for them.
pub(crate) const CLOCK_SET_AFTER: i64 = 1_704_067_200;

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

/// What a panel's glyph in Home's status corner shows (a `corner`
/// template): its badge, while the panel needs attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelCorner {
    pub props: Props,
}

/// What a panel's line on the idle screen shows (an `ambient` template).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelAmbient {
    pub props: Props,
}

/// A banner a panel shows once over Home (a `banner` template): an event,
/// not a state, so it comes from the change that makes it
/// ([`about_banner`], [`remote_banner`], [`history_banner`]), not from a
/// view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelBanner {
    pub props: Props,
}

/// Which faces a panel has: `None` for a face the panel doesn't have. About
/// has a tile, a row and a page, and Home relies on it (tests check it): the
/// About page and Settings' RustTV row are always there. RustTV Remote has
/// all three too; History a tile and a row (its page is Settings' History
/// section). The rows also make the RustTV quick panel (PANELS.md § 2).
/// Each also has a status-corner glyph and an idle-screen line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelView {
    pub tile: Option<PanelTile>,
    pub row: Option<PanelRow>,
    pub page: Option<PanelPage>,
    pub corner: Option<PanelCorner>,
    pub ambient: Option<PanelAmbient>,
}

/// About (templates/about_*.json). The tile shows the version, and a caption
/// while an update is available, downloading or installing; anything else, a
/// failure included, shows no caption (the reason is on the page). The row
/// shows the version and the tile's caption (else the licence line), with
/// the accent dot while an update is available. The page shows the version and the updater's state
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
            props: Props(vec![
                ("version", version.to_owned()),
                ("status", caption.to_owned()),
            ]),
        }),
        page: Some(PanelPage {
            props: Props(vec![
                ("version", version.to_owned()),
                ("update", update.to_owned()),
                ("update-detail", detail),
            ]),
        }),
        // The corner's glyph and the idle screen's line while an update is
        // available (`update`); not while it downloads or installs, which
        // the person started.
        corner: Some(PanelCorner {
            props: Props(vec![("status", caption.to_owned())]),
        }),
        ambient: Some(PanelAmbient {
            props: Props(vec![("status", caption.to_owned())]),
        }),
    }
}

/// About's banner when the updater's status changes from `before` to
/// `after`: an update became available (its version), or it was installed
/// (`version`, the running one); `None` for any other change. The first
/// status read since start (`before` is `Idle`) makes no "available"
/// banner: that update is old news, and the status corner's glyph shows it.
pub(crate) fn about_banner(
    version: &str,
    before: &UpdateStatus,
    after: &UpdateStatus,
) -> Option<PanelBanner> {
    let (event, version) = match after {
        UpdateStatus::Available(new) | UpdateStatus::NeedsPermission(new)
            if !before.is_available() && *before != UpdateStatus::Idle =>
        {
            ("available", new.as_str())
        }
        UpdateStatus::Installed if *before != UpdateStatus::Installed => ("installed", version),
        _ => return None,
    };
    Some(PanelBanner {
        props: Props(vec![
            ("event", event.to_owned()),
            ("version", version.to_owned()),
        ]),
    })
}

/// RustTV Remote (templates/remote_*.json). The tile and the row show how
/// many phones are paired, or "Pairing" (in the accent) while pairing is
/// open; the row is in the RustTV quick panel, not in Settings' list. The
/// page has no values of its own: its rows are Home's (ui.rs), as is its
/// status line.
pub(crate) fn remote_view(phones: usize, pairing: bool) -> PanelView {
    let props = || {
        Props(vec![
            ("phones", phones.to_string()),
            ("status", if pairing { "pairing" } else { "" }.to_owned()),
        ])
    };
    let status = || {
        Props(vec![(
            "status",
            if pairing { "pairing" } else { "" }.to_owned(),
        )])
    };
    PanelView {
        tile: Some(PanelTile { props: props() }),
        row: Some(PanelRow { props: props() }),
        page: Some(PanelPage {
            props: Props(Vec::new()),
        }),
        corner: Some(PanelCorner { props: status() }),
        ambient: Some(PanelAmbient { props: status() }),
    }
}

/// RustTV Remote's banner when a paired phone connects (`name`, as it was
/// paired), while RustTV is in front.
pub(crate) fn remote_banner(name: &str) -> PanelBanner {
    PanelBanner {
        props: Props(vec![("phone", name.to_owned())]),
    }
}

/// History (templates/history_*.json). `status` is "waiting" while an
/// approval waits for the computer (the tile's caption and the row's dot
/// in the accent), else the last change's age: "now" (under a minute),
/// "minutes", "hours" or "days", with that number as `count`; "" (the
/// tile's plain state) before the history thread has read the history
/// ([`Summary`], `None`), for an empty history, or when the TV's clock
/// isn't set or is behind the entry. `now_unix` is the TV's clock: Home
/// formats the age when it reads the summary (resume, focus changes),
/// never on a timer (PANELS.md § 4).
pub(crate) fn history_view(summary: Option<&Summary>, now_unix: i64) -> PanelView {
    let (status, count) = match summary {
        Some(summary) if summary.pending > 0 => ("waiting", 0),
        Some(Summary {
            last_unix: Some(last),
            ..
        }) if *last >= CLOCK_SET_AFTER && now_unix >= *last => {
            let seconds = now_unix - last;
            match seconds {
                0..60 => ("now", 0),
                60..3_600 => ("minutes", seconds / 60),
                3_600..86_400 => ("hours", seconds / 3_600),
                _ => ("days", seconds / 86_400),
            }
        }
        _ => ("", 0),
    };
    let props = || {
        Props(vec![
            ("status", status.to_owned()),
            ("count", count.to_string()),
        ])
    };
    PanelView {
        tile: Some(PanelTile { props: props() }),
        row: Some(PanelRow { props: props() }),
        page: None,
        // The corner's glyph while an approval waits; the idle screen's
        // line names the waiting approval or the last change's age.
        corner: Some(PanelCorner {
            props: Props(vec![("status", status.to_owned())]),
        }),
        ambient: Some(PanelAmbient { props: props() }),
    }
}

/// History's banner when the summary changes from `before` to `after`: the
/// computer applied approvals that waited for it (fewer wait, and a newer
/// entry recorded them), with how many. Approvals that ran out (two hours)
/// make no entry, so they make no banner either.
pub(crate) fn history_banner(
    before: Option<&Summary>,
    after: Option<&Summary>,
) -> Option<PanelBanner> {
    let (before, after) = (before?, after?);
    let applied = before
        .pending
        .checked_sub(after.pending)
        .filter(|n| *n > 0)?;
    let newer = match (before.last_unix, after.last_unix) {
        (Some(old), Some(new)) => new > old,
        (None, Some(_)) => true,
        (_, None) => false,
    };
    newer.then(|| PanelBanner {
        props: Props(vec![("count", applied.to_string())]),
    })
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

/// About's page template's paragraphs, in drawing order (start, then end).
#[cfg(test)]
fn paragraphs(page: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    ["start", "end"]
        .into_iter()
        .filter_map(|side| page["root"]["page"][side].as_array())
        .flatten()
        .filter_map(|block| block.get("paragraph"))
}

/// About's page template's updater line: the paragraph that selects on
/// `update`.
#[cfg(test)]
pub(crate) fn updater_line(page: &serde_json::Value) -> &serde_json::Value {
    paragraphs(page)
        .find(|paragraph| paragraph["value"]["select"] == "update")
        .expect("the updater's line")
}

/// Which drawn paragraph (`PageText`) is the updater line. Counts every
/// paragraph before it, so none of them may have a `show` (checked).
#[cfg(test)]
pub(crate) fn updater_line_index(page: &serde_json::Value) -> usize {
    let before: Vec<&serde_json::Value> = paragraphs(page)
        .take_while(|paragraph| paragraph["value"]["select"] != "update")
        .collect();
    assert!(
        before
            .iter()
            .all(|paragraph| paragraph.get("show").is_none()),
        "a conditional paragraph before the updater line moves it"
    );
    before.len()
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

    /// The row (Settings' RustTV row, the RustTV quick panel) shows the
    /// tile's caption in the tile's words, the licence line without one,
    /// and the accent dot only while an update is available.
    #[test]
    fn about_row_shows_the_tiles_caption() {
        for status in &STATUSES {
            let view = about_view("1.2.3", status);
            let tile = view.tile.expect("tile").props;
            let row = view.row.expect("row").props;
            assert_eq!(tile.get("status"), row.get("status"), "{status:?}");
        }
        let tile = template("about_tile");
        let caption = tile["root"]["column"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .filter_map(|child| child.get("text"))
            .find(|text| text["show"] == "status")
            .expect("the caption")["value"]["cases"]
            .clone();
        let row = &template("about_row")["root"]["list-row"];
        assert_eq!(row["detail"]["select"], "status");
        assert_eq!(row["detail"]["cases"], caption);
        assert_eq!(row["detail"]["else"]["tr"], "Third-party notices");
        assert_eq!(
            row["attention"]["cases"],
            serde_json::json!({"update": true})
        );
    }

    /// The corner's glyph and the idle screen's line get the tile's caption
    /// (a name means one value on every face); their templates show them
    /// only for "update", not while an update downloads or installs, which
    /// the person started (ui.rs tests draw them).
    #[test]
    fn about_corner_and_ambient_follow_the_caption() {
        for status in &STATUSES {
            let view = about_view("1.2.3", status);
            let caption = view.tile.expect("tile").props;
            let corner = view.corner.expect("corner").props;
            let ambient = view.ambient.expect("ambient").props;
            assert_eq!(corner.get("status"), caption.get("status"), "{status:?}");
            assert_eq!(ambient.get("status"), caption.get("status"), "{status:?}");
        }
        for face in ["about_corner", "about_ambient"] {
            let root = &template(face)["root"];
            let body = root.get("glyph").or(root.get("glance")).expect("root");
            assert_eq!(
                body["show"],
                serde_json::json!({"select": "status", "cases": {"update": true}}),
                "{face}"
            );
        }
    }

    #[test]
    fn about_banner_comes_once_for_a_new_update_and_for_an_install() {
        let available = UpdateStatus::Available("2.0.0".into());
        let banner =
            about_banner("1.2.3", &UpdateStatus::Checking, &available).expect("a new update");
        assert_eq!(banner.props.get("event"), "available");
        assert_eq!(banner.props.get("version"), "2.0.0");
        // Known at start already: no banner (the corner shows it).
        assert_eq!(about_banner("1.2.3", &UpdateStatus::Idle, &available), None);
        // Still available (or now needing permission): no second banner.
        assert_eq!(about_banner("1.2.3", &available, &available), None);
        let permission = UpdateStatus::NeedsPermission("2.0.0".into());
        assert_eq!(about_banner("1.2.3", &available, &permission), None);
        let installed = about_banner("1.2.3", &UpdateStatus::Installing, &UpdateStatus::Installed)
            .expect("installed");
        assert_eq!(installed.props.get("event"), "installed");
        assert_eq!(installed.props.get("version"), "1.2.3");
        assert_eq!(
            about_banner("1.2.3", &UpdateStatus::Installed, &UpdateStatus::Installed),
            None
        );
        for before in &STATUSES {
            for after in &STATUSES {
                let banner = about_banner("1.2.3", before, after);
                let expected = (after.is_available()
                    && !before.is_available()
                    && *before != UpdateStatus::Idle)
                    || (*after == UpdateStatus::Installed && *before != UpdateStatus::Installed);
                assert_eq!(banner.is_some(), expected, "{before:?} -> {after:?}");
            }
        }
    }

    #[test]
    fn remote_corner_and_ambient_show_pairing() {
        for pairing in [false, true] {
            let view = remote_view(2, pairing);
            let expected = if pairing { "pairing" } else { "" };
            assert_eq!(view.corner.expect("corner").props.get("status"), expected);
            assert_eq!(view.ambient.expect("ambient").props.get("status"), expected);
        }
        assert_eq!(remote_banner("Pixel").props.get("phone"), "Pixel");
    }

    #[test]
    fn history_corner_waits_and_ambient_carries_the_age() {
        let now = CLOCK_SET_AFTER + 10_000;
        let waiting = Summary {
            last_unix: Some(now - 7_200),
            pending: 1,
        };
        let view = history_view(Some(&waiting), now);
        assert_eq!(view.corner.expect("corner").props.get("status"), "waiting");
        let aged = Summary {
            pending: 0,
            ..waiting
        };
        let view = history_view(Some(&aged), now);
        assert_eq!(view.corner.expect("corner").props.get("status"), "hours");
        let ambient = view.ambient.expect("ambient").props;
        assert_eq!(ambient.get("status"), "hours");
        assert_eq!(ambient.get("count"), "2");
    }

    /// The banner only when the computer applied waiting approvals (a newer
    /// entry), never for approvals that ran out or before the first read.
    #[test]
    fn history_banner_counts_approvals_the_computer_applied() {
        let before = Summary {
            last_unix: Some(CLOCK_SET_AFTER),
            pending: 3,
        };
        let applied = Summary {
            last_unix: Some(CLOCK_SET_AFTER + 60),
            pending: 1,
        };
        let banner = history_banner(Some(&before), Some(&applied)).expect("applied");
        assert_eq!(banner.props.get("count"), "2");
        let ran_out = Summary {
            pending: 0,
            ..before
        };
        assert_eq!(history_banner(Some(&before), Some(&ran_out)), None);
        assert_eq!(history_banner(None, Some(&applied)), None);
        assert_eq!(history_banner(Some(&applied), Some(&applied)), None);
        let more = Summary {
            last_unix: Some(CLOCK_SET_AFTER + 120),
            pending: 4,
        };
        assert_eq!(history_banner(Some(&before), Some(&more)), None);
        let empty = Summary {
            last_unix: None,
            pending: 1,
        };
        let first = Summary {
            last_unix: Some(CLOCK_SET_AFTER),
            pending: 0,
        };
        assert!(history_banner(Some(&empty), Some(&first)).is_some());
    }

    /// Each face's template takes exactly the props its view sets, in
    /// order, so Rust can pass them by position (PANELS.md § 4.1).
    #[test]
    fn new_faces_props_match_their_templates() {
        let now = CLOCK_SET_AFTER;
        let views = [
            ("about", about_view("1", &UpdateStatus::Idle)),
            ("remote", remote_view(0, false)),
            ("history", history_view(None, now)),
        ];
        for (panel, view) in views {
            for (face, props) in [
                ("corner", view.corner.map(|face| face.props)),
                ("ambient", view.ambient.map(|face| face.props)),
            ] {
                let props = props.expect("every panel has the face");
                let names: Vec<&str> = props.0.iter().map(|(name, _)| *name).collect();
                let template = template(&format!("{panel}_{face}"));
                assert_eq!(
                    template["props"],
                    serde_json::json!(names),
                    "{panel} {face}"
                );
            }
        }
        let banners = [
            (
                "about",
                about_banner("1", &UpdateStatus::Checking, &UpdateStatus::Installed),
            ),
            ("remote", Some(remote_banner("P"))),
            (
                "history",
                history_banner(
                    Some(&Summary {
                        last_unix: None,
                        pending: 1,
                    }),
                    Some(&Summary {
                        last_unix: Some(now),
                        pending: 0,
                    }),
                ),
            ),
        ];
        for (panel, banner) in banners {
            let props = banner.expect("a banner").props;
            let names: Vec<&str> = props.0.iter().map(|(name, _)| *name).collect();
            let template = template(&format!("{panel}_banner"));
            assert_eq!(
                template["props"],
                serde_json::json!(names),
                "{panel} banner"
            );
        }
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

    #[test]
    fn remote_tile_shows_the_phones_or_pairing() {
        for (phones, pairing, count, status) in [
            (0, false, "0", ""),
            (1, false, "1", ""),
            (12, false, "12", ""),
            (2, true, "2", "pairing"),
        ] {
            let tile = remote_view(phones, pairing)
                .tile
                .expect("Remote has a tile");
            assert_eq!(tile.props.get("phones"), count);
            assert_eq!(tile.props.get("status"), status);
        }
    }

    /// Remote's view sets exactly the properties its templates declare, its
    /// tile and row the same values, and its tile and row have words for
    /// every status the view names ("" shows no caption) and for no phones,
    /// and no others.
    #[test]
    fn remote_view_matches_its_templates() {
        let declared = |name: &str| -> Vec<String> {
            template(name)["props"]
                .as_array()
                .expect("props")
                .iter()
                .map(|prop| prop.as_str().expect("string").to_owned())
                .collect()
        };
        for pairing in [false, true] {
            let view = remote_view(3, pairing);
            let tile = view.tile.expect("tile").props;
            let row = view.row.expect("row").props;
            assert_eq!(tile, row, "a name means the same on every face");
            assert_eq!(names(&tile), declared("remote_tile"));
            assert_eq!(names(&row), declared("remote_row"));
            assert_eq!(
                names(&view.page.expect("page").props),
                declared("remote_page")
            );
        }
        let tile = template("remote_tile");
        let texts: Vec<&serde_json::Value> = tile["root"]["column"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .filter_map(|child| child.get("text"))
            .collect();
        // No phones has its own words; any other count is the "Phones" line.
        let value = &texts
            .iter()
            .find(|text| text["value"]["select"] == "phones")
            .expect("the phones line")["value"];
        let counts: Vec<&String> = value["cases"].as_object().expect("cases").keys().collect();
        assert_eq!(counts, ["0"]);
        assert_eq!(value["else"]["tr"], "Phones: {}");
        assert_eq!(value["else"]["args"], serde_json::json!(["phones"]));
        let caption = texts
            .iter()
            .find(|text| text["show"] == "status")
            .expect("the caption");
        assert_eq!(caption["value"]["select"], "status");
        let cases: Vec<&String> = caption["value"]["cases"]
            .as_object()
            .expect("cases")
            .keys()
            .collect();
        assert_eq!(cases, ["pairing"]);
        // The row: "Pairing" first, else the same phones line as the tile.
        let row = &template("remote_row")["root"]["list-row"];
        let detail = &row["detail"];
        assert_eq!(detail["select"], "status");
        let cases: Vec<&String> = detail["cases"].as_object().expect("cases").keys().collect();
        assert_eq!(cases, ["pairing"]);
        assert_eq!(
            detail["cases"]["pairing"],
            caption["value"]["cases"]["pairing"]
        );
        assert_eq!(detail["else"], *value, "the tile's phones line");
        assert_eq!(
            row["attention"]["cases"],
            serde_json::json!({"pairing": true})
        );
    }

    const NOW: i64 = 1_790_000_000;

    fn history(summary: Option<Summary>, now: i64) -> (String, String) {
        let view = history_view(summary.as_ref(), now);
        let tile = view.tile.expect("History has a tile").props;
        let row = view.row.expect("History has a row").props;
        assert_eq!(tile, row, "a name means the same on every face");
        assert!(view.page.is_none(), "its page is Settings' History section");
        (tile.get("status").to_owned(), tile.get("count").to_owned())
    }

    fn last(seconds_ago: i64) -> Option<Summary> {
        at(NOW - seconds_ago)
    }

    /// A summary whose newest entry was made at `unix`.
    fn at(unix: i64) -> Option<Summary> {
        Some(Summary {
            last_unix: Some(unix),
            pending: 0,
        })
    }

    /// The age's unit and number at every boundary; the plain state before
    /// the summary arrives, for an empty history and a clock that isn't
    /// set; "waiting" over any age while an approval waits.
    #[test]
    fn history_tile_shows_the_age_or_an_approval_waiting() {
        let cases = [
            (None, NOW, "", "0"),
            (Some(Summary::default()), NOW, "", "0"),
            (last(0), NOW, "now", "0"),
            (last(59), NOW, "now", "0"),
            (last(60), NOW, "minutes", "1"),
            (last(3_599), NOW, "minutes", "59"),
            (last(3_600), NOW, "hours", "1"),
            (last(7_300), NOW, "hours", "2"),
            (last(86_399), NOW, "hours", "23"),
            (last(86_400), NOW, "days", "1"),
            (last(40 * 86_400), NOW, "days", "40"),
            // The TV's clock behind the entry; an entry made while the
            // clock wasn't set (before CLOCK_SET_AFTER), whatever it says
            // now; one made right at it counts.
            (last(-5), NOW, "", "0"),
            (at(100), NOW, "", "0"),
            (at(CLOCK_SET_AFTER - 1), NOW, "", "0"),
            (at(CLOCK_SET_AFTER), CLOCK_SET_AFTER + 30, "now", "0"),
            (
                Some(Summary {
                    last_unix: Some(NOW - 7_300),
                    pending: 2,
                }),
                NOW,
                "waiting",
                "0",
            ),
            (
                Some(Summary {
                    last_unix: None,
                    pending: 1,
                }),
                NOW,
                "waiting",
                "0",
            ),
        ];
        for (summary, now, status, count) in cases {
            assert_eq!(
                history(summary, now),
                (status.to_owned(), count.to_owned()),
                "{summary:?} at {now}"
            );
        }
    }

    /// History's view sets exactly the properties its templates declare;
    /// its tile's caption and its row's detail have words for every status
    /// the view names and no others, and the tile says "Last change" for
    /// every age. (The phone's words: remote.rs `history_caption`.)
    #[test]
    fn history_view_matches_its_templates() {
        let declared = |name: &str| -> Vec<String> {
            template(name)["props"]
                .as_array()
                .expect("props")
                .iter()
                .map(|prop| prop.as_str().expect("string").to_owned())
                .collect()
        };
        let view = history_view(last(10).as_ref(), NOW);
        assert_eq!(
            names(&view.tile.expect("tile").props),
            declared("history_tile")
        );
        assert_eq!(
            names(&view.row.expect("row").props),
            declared("history_row")
        );
        // Every status the view names, from the view itself.
        let waiting = Some(Summary {
            last_unix: None,
            pending: 1,
        });
        let mut named: Vec<String> = [last(0), last(120), last(7_200), last(172_800), waiting]
            .iter()
            .map(|summary| history(*summary, NOW).0)
            .collect();
        named.sort_unstable();
        assert_eq!(named, ["days", "hours", "minutes", "now", "waiting"]);
        let ages: Vec<&String> = named.iter().filter(|status| *status != "waiting").collect();
        let cases = |value: &serde_json::Value| -> Vec<String> {
            let mut cases: Vec<String> = value["cases"]
                .as_object()
                .expect("cases")
                .keys()
                .cloned()
                .collect();
            cases.sort_unstable();
            cases
        };
        let tile = template("history_tile");
        let caption = tile["root"]["column"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .filter_map(|child| child.get("text"))
            .find(|text| text["show"] == "status")
            .expect("the caption");
        assert_eq!(cases(&caption["value"]), named);
        let value = tile["root"]["column"]["children"]
            .as_array()
            .expect("children")
            .iter()
            .filter_map(|child| child.get("text"))
            .find(|text| text.get("show").is_none())
            .expect("the value line");
        assert_eq!(cases(&value["value"]).iter().collect::<Vec<_>>(), ages);
        for age in &ages {
            assert_eq!(value["value"]["cases"][age.as_str()]["tr"], "Last change");
        }
        assert_eq!(value["value"]["else"]["tr"], "History");
        let row = template("history_row");
        assert_eq!(cases(&row["root"]["list-row"]["detail"]), named);
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

    /// Every text a template shows, as `@tr` message ids, with its English
    /// plural for a text with plural forms.
    fn texts(value: &serde_json::Value, out: &mut Vec<(String, Option<String>)>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(text)) = map.get("tr") {
                    let plural = map
                        .get("plural")
                        .and_then(|p| p.as_str())
                        .map(str::to_owned);
                    out.push((text.clone(), plural));
                }
                map.values().for_each(|v| texts(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| texts(v, out)),
            _ => {}
        }
    }

    /// One gettext entry, still escaped: its msgid, its msgid_plural (if
    /// any) and its msgstr, or each msgstr[n] in order.
    #[derive(Debug, Default)]
    struct Entry {
        id: String,
        plural: Option<String>,
        strs: Vec<String>,
    }

    /// Which string a `"…"` continuation line extends.
    #[derive(Clone, Copy)]
    enum Field {
        Id,
        Plural,
        Str(usize),
    }

    /// A gettext file's entries, with wrapped lines (`msgid ""` then `"…"`
    /// lines) joined.
    fn entries(file: &str) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        let mut last = None;
        for line in file.lines() {
            let (field, rest) = if let Some(rest) = line.strip_prefix("msgid ") {
                out.push(Entry::default());
                (Some(Field::Id), rest)
            } else if let Some(rest) = line.strip_prefix("msgid_plural ") {
                (Some(Field::Plural), rest)
            } else if let Some(rest) = line.strip_prefix("msgstr ") {
                (Some(Field::Str(0)), rest)
            } else if let Some((index, rest)) = line
                .strip_prefix("msgstr[")
                .and_then(|rest| rest.split_once("] "))
            {
                (index.parse().ok().map(Field::Str), rest)
            } else if line.starts_with('"') {
                (last, line)
            } else {
                last = None;
                continue;
            };
            last = field;
            let text = rest
                .trim()
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .unwrap_or_default();
            let (Some(field), Some(entry)) = (field, out.last_mut()) else {
                continue;
            };
            match field {
                Field::Id => entry.id.push_str(text),
                Field::Plural => entry.plural.get_or_insert_default().push_str(text),
                Field::Str(index) => {
                    if entry.strs.len() <= index {
                        entry.strs.resize(index + 1, String::new());
                    }
                    entry.strs[index].push_str(text);
                }
            }
        }
        out
    }

    /// How many plural forms each language has (gettext's, as its header
    /// must say): Arabic six; Polish, Russian and Ukrainian three; French
    /// and Brazilian Portuguese two (0 and 1 are singular); one where a
    /// number doesn't change the noun.
    const PLURAL_FORMS: [(&str, usize); 20] = [
        ("ar", 6),
        ("de", 2),
        ("es", 2),
        ("fr", 2),
        ("he", 2),
        ("hi", 2),
        ("id", 1),
        ("it", 2),
        ("ja", 1),
        ("ko", 1),
        ("nl", 2),
        ("pl", 3),
        ("pt_BR", 2),
        ("ru", 3),
        ("th", 1),
        ("tr", 2),
        ("uk", 3),
        ("vi", 1),
        ("zh_CN", 1),
        ("zh_TW", 1),
    ];

    /// The number of plural forms a `.po` file's header names.
    fn plural_forms(entries: &[Entry]) -> usize {
        let header = &entries.first().expect("a header").strs[0];
        let forms = header
            .split("nplurals=")
            .nth(1)
            .expect("Plural-Forms names nplurals");
        forms
            .split(';')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .expect("a number of forms")
    }

    #[test]
    fn po_entries_read_plural_forms() {
        let po = "msgid \"\"\nmsgstr \"\"\n\"Plural-Forms: nplurals=3; plural=0;\\n\"\n\n\
                  msgid \"{n} day\"\nmsgid_plural \"{n} days\"\nmsgstr[0] \"a\"\nmsgstr[1] \"\"\n\"b\"\nmsgstr[2] \"c\"\n\n\
                  msgid \"One\"\nmsgstr \"Uno\"\n";
        let entries = entries(po);
        assert_eq!(plural_forms(&entries), 3);
        assert_eq!(entries[1].id, "{n} day");
        assert_eq!(entries[1].plural.as_deref(), Some("{n} days"));
        assert_eq!(entries[1].strs, ["a", "b", "c"]);
        assert_eq!(entries[2].plural, None);
        assert_eq!(entries[2].strs, ["Uno"]);
    }

    /// Template texts reach translators (scripts/launcher-pot.sh) and every
    /// language translates them: generated Slint lives in OUT_DIR, where a
    /// plain extractor run over ui/ would miss it. A text with plural forms
    /// has every form its language's header names, none empty.
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
        assert!(ids.iter().any(|(_, plural)| plural.is_some()));
        // Template texts hold no quotes or backslashes (convert.rs would
        // escape them), so their escaped form is the text itself.
        assert!(ids.iter().all(|(id, plural)| {
            !id.contains(['"', '\\']) && !plural.as_deref().unwrap_or("").contains(['"', '\\'])
        }));
        let pot =
            std::fs::read_to_string(dir.join("translations/rusttv-launcher.pot")).expect("pot");
        let pot = entries(&pot);
        for (id, plural) in &ids {
            let entry = pot
                .iter()
                .find(|entry| entry.id == *id)
                .unwrap_or_else(|| panic!("{id:?} missing from the .pot"));
            assert_eq!(entry.plural, *plural, "{id:?}'s plural in the .pot");
        }
        for language in crate::i18n::LANGUAGES {
            let po = dir.join(format!(
                "translations/{language}/LC_MESSAGES/rusttv-launcher.po"
            ));
            let po = std::fs::read_to_string(po).expect("po");
            // A fuzzy entry isn't used: it must not stand in for one.
            assert!(!po.contains("#, fuzzy"), "{language}: a fuzzy entry");
            let po = entries(&po);
            let forms = plural_forms(&po);
            let want_forms = PLURAL_FORMS
                .iter()
                .find(|(code, _)| code == language)
                .map(|(_, forms)| *forms);
            assert_eq!(Some(forms), want_forms, "{language}: Plural-Forms");
            for (id, plural) in &ids {
                let entry = po
                    .iter()
                    .find(|entry| entry.id == *id)
                    .unwrap_or_else(|| panic!("{language}: {id:?} missing"));
                assert_eq!(entry.plural, *plural, "{language}: {id:?}'s plural");
                let want = if plural.is_some() { forms } else { 1 };
                assert_eq!(entry.strs.len(), want, "{language}: {id:?}'s forms");
                assert!(
                    entry.strs.iter().all(|text| !text.is_empty()),
                    "{language}: {id:?} is not translated"
                );
                // Some forms may say the number in words ("an hour"), but
                // the number shows in one at least.
                if plural.is_some() {
                    assert!(
                        entry.strs.iter().any(|text| text.contains("{n}")),
                        "{language}: {id:?} never shows {{n}}"
                    );
                }
            }
        }
    }
}
