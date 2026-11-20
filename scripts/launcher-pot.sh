#!/bin/sh
# Regenerates the launcher's translation template from every @tr(...): the
# hand-written UI (ui/home.slint, ui/quick.slint, ui/blocks.slint) and the
# Slint that build.rs generates from the panel templates (templates/*.json,
# PANELS.md § 4.1).
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
set -- ui/home.slint ui/quick.slint ui/blocks.slint
for template in templates/*.json; do
    set -- "$@" "$out/$(basename "$template" .json).slint"
done
slint-tr-extractor --no-default-translation-context "$@" -o translations/rusttv-launcher.pot
# Point each generated file's references at its template, not at this
# machine's build folder (one reference per line or several on one line).
python3 - "$out" <<'PY'
import glob, json, re, sys
out = re.escape(sys.argv[1].rstrip("/"))
path = "translations/rusttv-launcher.pot"
text = open(path, encoding="utf-8").read()
text = re.sub(out + r"/([a-z_]+)\.slint:\d+", r"templates/\1.json", text)
# For a text used in two files the extractor writes the first file's
# reference twice and loses the second; keep one copy.
text = re.sub(r"^(#: [^\n]*\n)\1+", r"\1", text, flags=re.M)
# Then name every file that shows a text: each template whose JSON has it
# as a `tr` (a plural text by its singular), each `@tr("…"` line in ui/.
def texts(value, found):
    if isinstance(value, dict):
        if isinstance(value.get("tr"), str):
            found.add(value["tr"])
        for item in value.values():
            texts(item, found)
    elif isinstance(value, list):
        for item in value:
            texts(item, found)
templates = []
for template in sorted(glob.glob("templates/*.json")):
    found = set()
    texts(json.load(open(template, encoding="utf-8")), found)
    templates.append((template, found))
ui = [(name, open(name, encoding="utf-8").read().split("\n"))
      for name in ("ui/home.slint", "ui/quick.slint", "ui/blocks.slint")]
def unescape(escaped):
    return re.sub(r'\\(.)', lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), escaped)
def references(block):
    msgid = re.search(r'^msgid ((?:"(?:[^"\\]|\\.)*"\n?)+)', block, re.M)
    if not msgid:
        return None
    escaped = "".join(re.findall(r'"((?:[^"\\]|\\.)*)"', msgid.group(1)))
    if not escaped:
        return None
    refs = [f"{name}:{number}" for name, lines in ui
            for number, line in enumerate(lines, 1) if f'@tr("{escaped}"' in line]
    refs += [template for template, found in templates if unescape(escaped) in found]
    return refs
blocks = text.split("\n\n")
for index, block in enumerate(blocks):
    refs = references(block)
    if refs:
        lines = [line for line in block.split("\n") if not line.startswith("#: ")]
        blocks[index] = "\n".join(["#: " + " ".join(refs)] + lines)
text = "\n\n".join(blocks)
open(path, "w", encoding="utf-8").write(text)
PY
