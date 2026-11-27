// Standalone Coastal Dock Pro script (sandboxed Rhai).
// Up/Down moves between the bottom dock and top-right bar; 'm' toggles bar style.

fn on_init() {
    set_int(0, 0); // 0 = inline labels, 1 = bare icons
    set_int(1, 0); // 0 = bottom dock, 1 = top bar
    set_int(2, 1); // 0 = bar style toggle, 1 = Settings
    set_int(3, 0); // focused app index
    set_int(4, 0); // horizontal dock scroll offset
}

fn on_focus(index, id, label) {
    set_int(3, index);
    let scroll = get_int(4);
    if index < scroll {
        set_int(4, index);
    } else if index >= scroll + 6 {
        set_int(4, index - 5);
    }
}

fn on_action(name, arg) {
    if name == "settings" {
        open_settings();
    } else if name == "toggle_bar" {
        set_int(0, 1 - get_int(0));
    }
}

fn on_key(key, pressed) {
    if get_int(1) == 1 {
        if !pressed { return true; }
        let cur = get_int(2);
        if key == "Down" || key == "Back" {
            set_int(1, 0);
            return true;
        }
        if key == "Left" || key == "Right" {
            set_int(2, 1 - cur);
            return true;
        }
        if key == "Ok" {
            if cur == 1 {
                set_int(1, 0);
                open_settings();
            } else {
                set_int(0, 1 - get_int(0));
            }
            return true;
        }
        if key == "Up" { return true; }
        return false;
    }

    if !pressed { return false; }
    if key == "Left" {
        let cur = get_int(3);
        if cur > 0 { focus(cur - 1); }
        return true;
    }
    if key == "Right" {
        let cur = get_int(3);
        let max_idx = tile_count() - 1;
        if cur < max_idx { focus(cur + 1); }
        return true;
    }
    if key == "Down" {
        return true;
    }
    if key == "Up" {
        set_int(1, 1);
        set_int(2, 1);
        return true;
    }
    if key == "m" {
        set_int(0, 1 - get_int(0));
        return true;
    }
    false
}
