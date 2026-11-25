// Standalone Horizon Frame Pro script (sandboxed Rhai).
// State slots:
//   i0 = mode: 0 = Glass Dock, 1 = Photo List, 2 = All Apps sheet
//   i1 = focus zone: 0 = apps, 1 = top-right utilities
//   i2 = focused utility index: 0 = Layout toggle (Dock/List), 1 = All apps, 2 = Settings
//   i3 = focused app index
//   i4 = dock/list scroll offset (first visible tile index)
//   s0 = floating utility focus label ("List view" / "Dock view" / "All apps" / "Settings")

fn util_label(idx, mode) {
    if idx == 0 {
        if mode == 1 { return "Dock view"; }
        return "List view";
    }
    if idx == 1 {
        if mode == 2 { return "Home"; }
        return "All apps";
    }
    return "Settings";
}

fn sync_scroll(idx, mode) {
    let scroll = get_int(4);
    if mode == 0 {
        // 5 full tiles visible inside 1472px shelf (288px each + 6th peeking).
        if idx < scroll {
            set_int(4, idx);
        } else if idx >= scroll + 5 {
            set_int(4, idx - 4);
        }
    } else if mode == 1 {
        // Photo List: 5 rows visible, centered around focused row.
        let target = idx - 2;
        if target < 0 { target = 0; }
        let max_s = tile_count() - 5;
        if max_s < 0 { max_s = 0; }
        if target > max_s { target = max_s; }
        set_int(4, target);
    } else {
        // All Apps sheet: 5 columns x 3 visible rows.
        let row = idx / 5;
        if row < scroll {
            set_int(4, row);
        } else if row >= scroll + 3 {
            set_int(4, row - 2);
        }
    }
}

fn on_init() {
    let saved = load_pref("layout", "dock");
    let mode = if saved == "list" { 1 } else { 0 };
    set_int(0, mode);
    set_int(1, 0);
    set_int(2, 2);
    set_int(3, 0);
    set_int(4, 0);
    set_str(0, "Settings");
}

fn on_tiles_loaded() {
    // Skin-owned icon pipeline: extract crisp warm-white glyphs with 22% frosted
    // badge silhouettes directly in Rhai (no Rust changes needed to tweak style).
    let n = tile_count();
    let i = 0;
    while i < n {
        let src = tile_image(i);
        if src > 0 {
            let styled = img_extract_glyph(src, 0.22, "#F4F4F1");
            if styled > 0 {
                set_tile_image(i, styled);
            }
        }
        i += 1;
    }
}

fn on_focus(index, id, label) {
    set_int(3, index);
    sync_scroll(index, get_int(0));
}

fn trigger_util(idx) {
    let mode = get_int(0);
    if idx == 0 {
        let next_mode = if mode == 1 { 0 } else { 1 };
        set_int(0, next_mode);
        save_pref("layout", if next_mode == 1 { "list" } else { "dock" });
        set_int(4, 0);
        sync_scroll(get_int(3), next_mode);
        set_str(0, util_label(0, next_mode));
        set_int(1, 0);
    } else if idx == 1 {
        let next_mode = if mode == 2 { 0 } else { 2 };
        set_int(0, next_mode);
        set_int(4, 0);
        sync_scroll(get_int(3), next_mode);
        set_str(0, util_label(1, next_mode));
        set_int(1, 0);
    } else {
        set_int(1, 0);
        open_settings();
    }
}

fn on_action(name, arg) {
    if name == "util0" { trigger_util(0); }
    else if name == "util1" { trigger_util(1); }
    else if name == "util2" { trigger_util(2); }
}

fn on_key(key, pressed) {
    let mode = get_int(0);
    let zone = get_int(1);

    if zone == 1 {
        if !pressed { return true; }
        let u = get_int(2);
        if key == "Down" || key == "Back" {
            set_int(1, 0);
            return true;
        }
        if key == "Left" {
            let next = u - 1;
            if next < 0 { next = 2; }
            set_int(2, next);
            set_str(0, util_label(next, mode));
            return true;
        }
        if key == "Right" {
            let next = u + 1;
            if next > 2 { next = 0; }
            set_int(2, next);
            set_str(0, util_label(next, mode));
            return true;
        }
        if key == "Ok" {
            trigger_util(u);
            return true;
        }
        if key == "Up" { return true; }
        return false;
    }

    if !pressed { return false; }
    let cur = get_int(3);
    let max_idx = tile_count() - 1;

    if key == "Back" && mode != 0 {
        set_int(0, 0);
        set_int(4, 0);
        sync_scroll(cur, 0);
        return true;
    }

    if mode == 0 {
        // Glass Dock: 1D horizontal track
        if key == "Left" {
            if cur > 0 { focus(cur - 1); }
            return true;
        }
        if key == "Right" {
            if cur < max_idx { focus(cur + 1); }
            return true;
        }
        if key == "Down" { return true; }
        if key == "Up" {
            set_int(1, 1);
            set_str(0, util_label(get_int(2), mode));
            return true;
        }
    } else if mode == 1 {
        // Photo List: vertical typographic list
        if key == "Up" {
            if cur > 0 { focus(cur - 1); }
            return true;
        }
        if key == "Down" {
            if cur < max_idx { focus(cur + 1); }
            return true;
        }
        if key == "Right" {
            set_int(1, 1);
            set_str(0, util_label(get_int(2), mode));
            return true;
        }
        if key == "Left" { return true; }
    } else {
        // All Apps sheet: 5-column grid
        if key == "Left" {
            if cur > 0 { focus(cur - 1); }
            return true;
        }
        if key == "Right" {
            if cur < max_idx { focus(cur + 1); }
            return true;
        }
        if key == "Down" {
            let next = cur + 5;
            if next <= max_idx {
                focus(next);
            } else if (cur / 5) < (max_idx / 5) {
                focus(max_idx);
            }
            return true;
        }
        if key == "Up" {
            if cur >= 5 {
                focus(cur - 5);
                return true;
            }
            set_int(1, 1);
            set_str(0, util_label(get_int(2), mode));
            return true;
        }
    }
    false
}
