#!/bin/sh
# Regenerates the launcher's translation template from every @tr(...): the
# hand-written UI (ui/home.slint, ui/quick.slint) and the Slint that build.rs
# generates from the panel templates (templates/*.json, PANELS.md § 4.1).
# Needs slint-tr-extractor (`cargo install slint-tr-extractor`) and python3.
set -eu
cd "$(dirname "$0")/../crates/rusttv-launcher"
# Build so the generated files exist, then find this build's OUT_DIR (any
# CARGO_TARGET_DIR) through cargo's own message.
out=$(cargo build -p rusttv-launcher --message-format=json 2>/dev/null | python3 -c '
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("reason") == "build-script-executed" and "rusttv-launcher" in msg.get("package_id", ""):
        print(msg["out_dir"])
')
test -n "$out" || { echo "no OUT_DIR from cargo" >&2; exit 1; }
set -- ui/home.slint ui/quick.slint
for template in templates/*.json; do
    set -- "$@" "$out/$(basename "$template" .json).slint"
done
slint-tr-extractor --no-default-translation-context "$@" -o translations/rusttv-launcher.pot
# Point each generated file's references at its template, not at this
# machine's build folder (one reference per line or several on one line).
python3 - "$out" <<'PY'
import re, sys
out = re.escape(sys.argv[1].rstrip("/"))
path = "translations/rusttv-launcher.pot"
text = open(path, encoding="utf-8").read()
text = re.sub(out + r"/([a-z_]+)\.slint:\d+", r"templates/\1.json", text)
# The extractor repeats a reference line for a text used twice; keep one.
text = re.sub(r"^(#: [^\n]*\n)\1+", r"\1", text, flags=re.M)
open(path, "w", encoding="utf-8").write(text)
PY
