#!/bin/sh
# Renders the RustTV brand assets from the HTML templates in this folder with
# headless Chrome, then scales the banner with Python/Pillow.
# Needs: Google Chrome, python3 with Pillow, and Inter (SIL OFL 1.1), which
# ships in the i-slint-common crate (fetched by any Slint build).
set -eu
cd "$(dirname "$0")"
ROOT=../..
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
FONT=$(ls ~/.cargo/registry/src/*/i-slint-common-*/sharedfontique/Inter-VariableFont.ttf | head -1)
cp "$FONT" Inter-VariableFont.ttf
trap 'rm -f Inter-VariableFont.ttf' EXIT
OUT=$(mktemp -d)

shot() { # template width height output
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
        --default-background-color=00000000 --window-size="$2,$3" \
        --screenshot="$4" "file://$PWD/$1" >/dev/null 2>&1
}

shot icon.html 1024 1024 "$OUT/icon.png"
shot icon-background.html 432 432 "$OUT/bg.png"
shot icon-foreground.html 432 432 "$OUT/fg.png"
shot banner.html 960 540 "$OUT/banner.png"
shot logo.html 1280 640 "$OUT/logo.png"

python3 - "$OUT" "$ROOT" <<'PY'
import sys
from pathlib import Path
from PIL import Image
out, root = Path(sys.argv[1]), Path(sys.argv[2])
res = root / "android/app/src/main/res"
docs = root / "docs/brand"
docs.mkdir(parents=True, exist_ok=True)
Image.open(out / "icon.png").save(docs / "icon-1024.png", optimize=True)
Image.open(out / "icon.png").resize((512, 512), Image.LANCZOS).save(docs / "icon-512.png", optimize=True)
Image.open(out / "logo.png").convert("RGB").save(docs / "logo-1280x640.png", optimize=True)
# Android TV banner: 320x180 (xhdpi) from the 3x render.
(res / "drawable-xhdpi").mkdir(parents=True, exist_ok=True)
Image.open(out / "banner.png").convert("RGB").resize((320, 180), Image.LANCZOS).save(
    res / "drawable-xhdpi/banner.png", optimize=True)
# Adaptive icon layers per density (108dp canvas).
for name, px in [("mdpi", 108), ("hdpi", 162), ("xhdpi", 216), ("xxhdpi", 324), ("xxxhdpi", 432)]:
    folder = res / f"mipmap-{name}"
    folder.mkdir(parents=True, exist_ok=True)
    Image.open(out / "bg.png").convert("RGB").resize((px, px), Image.LANCZOS).save(
        folder / "ic_launcher_background.png", optimize=True)
    Image.open(out / "fg.png").resize((px, px), Image.LANCZOS).save(
        folder / "ic_launcher_foreground.png", optimize=True)
print("wrote docs/brand/{icon-1024,icon-512,logo-1280x640}.png, banner, adaptive icon layers")
PY
