// Classic Home Pro script (sandboxed Rhai).
// Drives the default RustTV home layout as a Level 3 skin: a 6-column grid of
// 16:9 cards whose first row is a shelf under the hero, 3 visible rows once
// focus leaves the shelf. Up from row 0 falls through to RustTV, which
// moves focus to its top-right menu (drawn by skin.slint).
// Numbers are inlined (Rhai functions don't see top-level constants):
// 6 = columns, 3 = visible rows while scrolled.
// State slots:
//   i3 = focused app index
//   i4 = top visible row while scrolled (grid mode)
//   i5 = 1 while focus is in row 0 (hero shown), else 0
//   i6 = tiles 0..i6 carry a baked, blurred glow picture in `mono-image`

fn on_init() {
    set_int(3, 0);
    set_int(4, 0);
    set_int(5, 1);
    set_int(6, 0);
}

// Bakes one soft, blurred copy of each tile's artwork once per tile refresh
// (into its `mono-image`). skin.slint draws it scaled up as the hero's colour
// glow, so nothing is blurred per frame. Three passes while the 128 script
// image handles allow (4 per tile), else two, else one.
fn on_tiles_loaded() {
    let n = tile_count();
    let passes = if n * 4 <= 128 { 3 } else if n * 3 <= 128 { 2 } else { 1 };
    let baked = 0;
    for i in 0..n {
        let b = tile_image(i);
        if b == 0 {
            baked = i + 1;
            continue;
        }
        for p in 0..passes {
            if b != 0 {
                b = img_blur(b, 24);
            }
        }
        if b == 0 {
            break;
        }
        set_tile_image(i, b);
        baked = i + 1;
    }
    set_int(6, baked);
}

// The default scroll rule: keep the focused row inside 3 slots, scrolling as
// little as possible; row 0 is the shelf.
fn on_focus(index, id, label) {
    set_int(3, index);
    let row = index / 6;
    let first = get_int(4);
    if row == 0 || row < first {
        first = row;
    } else if row >= first + 3 {
        first = row - 2;
    }
    set_int(4, first);
    set_int(5, if row == 0 { 1 } else { 0 });
}

// Down into a short last row lands on the last tile (model.rs `neighbour_in`).
fn move_down(cur, max_idx) {
    let below = cur + 6;
    if below <= max_idx {
        focus(below);
    } else if cur / 6 < max_idx / 6 {
        focus(max_idx);
    }
}

fn on_action(name, arg) {
    if name == "hover" {
        let i = parse_int(arg);
        if i != get_int(3) {
            focus(i);
        }
    } else if name == "wheel" {
        let cur = get_int(3);
        if arg == "down" {
            move_down(cur, tile_count() - 1);
        } else if cur >= 6 {
            focus(cur - 6);
        }
    }
}

// While RustTV's top-right menu or long-press menu has focus, RustTV handles
// the keys itself; this only runs for the grid.
fn on_key(key, pressed) {
    if !pressed {
        return false;
    }
    let cur = get_int(3);
    let max_idx = tile_count() - 1;
    let gcol = cur % 6;
    if key == "Left" {
        if gcol > 0 {
            focus(cur - 1);
        }
        return true;
    }
    if key == "Right" {
        if gcol + 1 < 6 && cur < max_idx {
            focus(cur + 1);
        }
        return true;
    }
    if key == "Down" {
        move_down(cur, max_idx);
        return true;
    }
    if key == "Up" {
        if cur >= 6 {
            focus(cur - 6);
            return true;
        }
        // From the first row: RustTV moves focus to its top-right menu.
        return false;
    }
    false
}
