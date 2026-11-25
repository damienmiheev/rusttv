// Standalone Ambient Pills Pro script (sandboxed Rhai).
// 4-column scrolling dock + top-right glass pill bar.

fn on_init() {
    set_int(1, 0); // 0 = app dock, 1 = top-right pill
    set_int(3, 0); // focused app index
    set_int(4, 0); // active visible row (each row has 4 cards)
}

fn on_focus(index, id, label) {
    set_int(3, index);
    set_int(4, index / 4);
}

fn on_action(name, arg) {
    if name == "settings" {
        open_settings();
    }
}

fn on_key(key, pressed) {
    if get_int(1) == 1 {
        if !pressed { return true; }
        if key == "Down" || key == "Back" {
            set_int(1, 0);
            return true;
        }
        if key == "Ok" {
            set_int(1, 0);
            open_settings();
            return true;
        }
        if key == "Up" || key == "Left" || key == "Right" {
            return true;
        }
        return false;
    }

    if !pressed { return false; }
    let cur = get_int(3);
    let max_idx = tile_count() - 1;
    if key == "Left" {
        if cur > 0 { focus(cur - 1); }
        return true;
    }
    if key == "Right" {
        if cur < max_idx { focus(cur + 1); }
        return true;
    }
    if key == "Down" {
        let next = cur + 4;
        if next <= max_idx {
            focus(next);
        } else if (cur / 4) < (max_idx / 4) {
            focus(max_idx);
        }
        return true;
    }
    if key == "Up" {
        if cur >= 4 {
            focus(cur - 4);
            return true;
        }
        set_int(1, 1);
        return true;
    }
    false
}
