// Pass bar 2 (PANELS.md § 4.1): the phone page draws the same template JSON
// the TV's Slint is generated from. Same schema, same rules: unknown fields,
// tokens, badges or props are errors, never guesses.
"use strict";

// Template token → the phone page's CSS variable (index.html :root). The
// page already has them all, under its own names.
const TOKENS = { bg: "--bg", surface: "--surface", text: "--text", "text-dim": "--dim", accent: "--accent" };

/**
 * Draws `template` into a new element.
 * values: the panel's props ({ version: "0.4.0", status: "update" }).
 * tr(msgid): the phone's translation of a launcher message id.
 * badges: badge name → image URL.
 * rtl: the phone's direction.
 */
export function render(template, { values, tr, badges, rtl }) {
  only(template, "template", ["component", "props", "root"]);
  const props = template.props;
  const prop = (name, at) => {
    if (!props.includes(name)) throw new Error(`${at}: ${JSON.stringify(name)} is not in props`);
    return values[name] ?? "";
  };
  const words = (value, at) => {
    if ("select" in value) return select(value, at, words, "");
    only(value, at, ["tr", "args"]);
    if (typeof value.tr !== "string" || /[{}]/.test(value.tr.replaceAll("{}", ""))) throw new Error(`${at}.tr: braces only as {}`);
    const args = (value.args || []).map((a, i) => prop(a, `${at}.args[${i}]`));
    if (args.length !== value.tr.split("{}").length - 1) throw new Error(`${at}: needs one arg per {}`);
    let i = 0;
    return tr(value.tr).replace(/\{\}/g, () => args[i++] ?? "");
  };
  const color = (value, at) => {
    if (typeof value === "object") return select(value, at, color, "var(--text)");
    if (!(value in TOKENS)) throw new Error(`${at}: ${JSON.stringify(value)} is not a colour token`);
    return `var(${TOKENS[value]})`;
  };
  const select = (value, at, each, fallback) => {
    only(value, at, ["select", "cases", "else"]);
    const current = prop(value.select, `${at}.select`);
    // Every case is checked, not only the one taken, as the converter does.
    const drawn = Object.entries(value.cases).map(([k, v]) => [k, each(v, `${at}.cases.${k}`)]);
    const other = "else" in value ? each(value.else, `${at}.else`) : fallback;
    return (drawn.find(([k]) => k === current) || [null, other])[1];
  };
  const u = (n, at) => {
    if (typeof n !== "number" || !(n >= 0 && n <= 4000)) throw new Error(`${at}: expected a number from 0 to 4000`);
    return `calc(${n} * var(--u))`;
  };

  const node = (value, at, root) => {
    const kinds = Object.keys(value);
    if (kinds.length !== 1) throw new Error(`${at}: a node has exactly one key, its kind`);
    const kind = kinds[0];
    const body = value[kind];
    at = `${at}.${kind}`;
    if (kind === "column") {
      only(body, at, root ? ["width", "height", "radius", "background", "padding", "gap", "align", "children"]
        : ["padding", "gap", "align", "children"]);
      const el = document.createElement("div");
      Object.assign(el.style, {
        display: "flex", flexDirection: "column", boxSizing: "border-box",
        justifyContent: { start: "flex-start", center: "center", end: "flex-end" }[body.align || "start"],
        padding: u(body.padding ?? 0, `${at}.padding`), gap: u(body.gap ?? 0, `${at}.gap`), minWidth: "0",
      });
      if (root) {
        Object.assign(el.style, {
          width: u(body.width, `${at}.width`), height: u(body.height, `${at}.height`), borderRadius: u(body.radius ?? 0, `${at}.radius`),
          background: body.background ? color(body.background, `${at}.background`) : "", overflow: "hidden",
        });
      }
      body.children.forEach((child, i) => {
        const drawn = node(child, `${at}.children[${i}]`, false);
        if (drawn) el.append(drawn);
      });
      return el;
    }
    if (kind === "text") {
      only(body, at, ["value", "size", "weight", "color", "show"]);
      const weight = body.weight ?? 400;
      if (!(Number.isInteger(weight) && weight >= 100 && weight <= 900 && weight % 100 === 0)) throw new Error(`${at}.weight: not 100, 200 … 900`);
      const text = words(body.value, `${at}.value`);
      const size = u(body.size, `${at}.size`);
      const fill = color(body.color, `${at}.color`);
      if ("show" in body && prop(body.show, `${at}.show`) === "") return null;
      const el = document.createElement("div");
      el.textContent = text;
      // As Slint: aligned to the UI's start side (Dir.start), while the
      // text's own script decides where "…" goes (dir=auto).
      el.dir = "auto";
      Object.assign(el.style, {
        fontSize: size, fontWeight: String(weight), color: fill,
        textAlign: rtl ? "right" : "left", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", lineHeight: "normal",
      });
      return el;
    }
    if (kind === "image") {
      only(body, at, ["badge", "height"]);
      if (!Object.hasOwn(badges, body.badge)) throw new Error(`${at}.badge: ${JSON.stringify(body.badge)} is not a built-in badge`);
      const el = document.createElement("img");
      el.src = badges[body.badge];
      Object.assign(el.style, { height: u(body.height, `${at}.height`), objectFit: "contain", alignSelf: "flex-start" });
      return el;
    }
    throw new Error(`${at}: unknown node ${JSON.stringify(kind)} (column, text, image)`);
  };

  if (Object.keys(template.root || {})[0] !== "column") throw new Error("root: must be a column");
  const el = node(template.root, "root", true);
  el.dir = rtl ? "rtl" : "ltr";
  return el;
}

function only(map, at, allowed) {
  for (const key of Object.keys(map)) {
    if (!allowed.includes(key)) throw new Error(`${at}: unknown field ${JSON.stringify(key)}`);
  }
}
