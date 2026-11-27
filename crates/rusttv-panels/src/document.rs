//! A panel's document (PANEL_STUDIO.md § 5.3): one variant per face, size
//! class and state, each a tree of nodes.
//!
//! Node kinds so far: frame, group, rectangle, ellipse, line, text, image,
//! progress and repeat (its children drawn once per list item), with literal
//! values and bindings: a node's `bindings` set some of its properties from
//! expressions, with the literal property as the fallback. Components,
//! vectors and widgets come with later slices.

use crate::error::{Code, PackageError, Position};
use crate::expr::{self, Expr, Scope, Type};
use crate::meta::{Face, Meta, Name, limits};
use crate::text::Text;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A colour, `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (255 = opaque).
    pub a: u8,
}

impl Color {
    /// The colour in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadColor`] unless `text` is `#` and 6 or 8 hex digits.
    pub fn parse(text: &str) -> Result<Self, Code> {
        let hex = text
            .strip_prefix('#')
            .filter(|h| matches!(h.len(), 6 | 8))
            .ok_or(Code::BadColor)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Code::BadColor);
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| Code::BadColor);
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Ok(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a,
        })
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}{:02x}",
            self.r, self.g, self.b, self.a
        )
    }
}

deserialize_via!(Color, Color::parse);

/// A node id, `[a-z0-9_-]{1,32}`, unique within its variant; the studio keeps
/// ids across edits so later bindings survive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// The id in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadNodeId`] unless `text` is `[a-z0-9_-]{1,32}`.
    pub fn parse(text: &str) -> Result<Self, Code> {
        ident(text, 32, Code::BadNodeId).map(Self)
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via!(NodeId, NodeId::parse);

/// An image's name in the package, `[a-z0-9_-]{1,40}`: the file
/// `assets/<name>.png` (checked against the zip in a later slice).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetName(String);

impl AssetName {
    /// The name in `text`.
    ///
    /// # Errors
    ///
    /// [`Code::BadAssetName`] unless `text` is `[a-z0-9_-]{1,40}`.
    pub fn parse(text: &str) -> Result<Self, Code> {
        ident(text, 40, Code::BadAssetName).map(Self)
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via!(AssetName, AssetName::parse);

fn ident(text: &str, max: usize, bad: Code) -> Result<String, Code> {
    let ok = (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok { Ok(text.to_owned()) } else { Err(bad) }
}

/// A number in `u` (the launcher's unit; the window is 1920 u wide) or
/// another unit, finite and inside its property's range.
fn in_range(value: f32, range: std::ops::RangeInclusive<f32>) -> bool {
    value.is_finite() && range.contains(&value)
}

/// A size class (PANEL_STUDIO.md § 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SizeClass {
    /// No larger than the face's reference size.
    Compact,
    /// The reference size.
    #[default]
    Regular,
    /// No smaller than the reference size, for big text scales.
    Large,
}

/// Which kind of node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum NodeKind {
    /// A container with children; may fill, round, stroke, shadow and clip.
    Frame,
    /// A container that draws nothing itself; may clip.
    Group,
    /// A filled rectangle.
    Rectangle,
    /// A filled ellipse in its box.
    Ellipse,
    /// A straight line across its box.
    Line,
    /// Text.
    Text,
    /// A PNG from the package.
    Image,
    /// A progress bar.
    Progress,
    /// Its children drawn once per item of a list (`list`), at most `max`
    /// times, each in a cell of `item_w`×`item_h` along `direction`.
    Repeat,
}

/// Horizontal text alignment, in reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Align {
    /// Where reading starts (left in LTR).
    Start,
    /// Centred.
    Center,
    /// Where reading ends.
    End,
}

/// Which way a repeat lays out its items, in reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Direction {
    /// One after another across (mirrored in right-to-left languages).
    Row,
    /// One below another.
    #[default]
    Column,
}

/// How an image fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Fit {
    /// All of it visible.
    Contain,
    /// The box filled, cropped.
    Cover,
    /// Stretched.
    Fill,
}

/// A stroke along a node's edge (inside, as the TV draws it).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Stroke {
    /// Its colour.
    pub color: Color,
    /// Its width in `u`, 0 to 20.
    pub width: f32,
}

/// A drop shadow.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Shadow {
    /// Offset right, in `u`, −200 to 200.
    #[serde(default)]
    pub x: f32,
    /// Offset down, in `u`, −200 to 200.
    #[serde(default)]
    pub y: f32,
    /// Blur radius in `u`, 0 to 100.
    pub blur: f32,
    /// Its colour.
    pub color: Color,
}

/// One node. Every kind shares the box and the common properties; each
/// kind-specific property is refused on other kinds ([`Code::NotForKind`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Node {
    /// Its id, unique in the variant.
    pub id: NodeId,
    /// Its kind.
    #[serde(rename = "type")]
    pub kind: NodeKind,
    /// Start edge in `u` from the parent's start edge: from the left in
    /// left-to-right languages, mirrored by the host in right-to-left ones.
    #[serde(default)]
    pub x: f32,
    /// Top edge in `u`, relative to the parent.
    #[serde(default)]
    pub y: f32,
    /// Width in `u`.
    pub w: f32,
    /// Height in `u`.
    pub h: f32,
    /// 0 (invisible) to 1.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Clockwise degrees, −360 to 360; needs the `rotation` feature.
    #[serde(default)]
    pub rotation: f32,
    /// What a screen reader says; none for decoration.
    #[serde(default)]
    pub label: Option<Text<{ limits::LABEL }>>,

    /// Frame, rectangle, ellipse: the fill. Progress: the track.
    #[serde(default)]
    pub fill: Option<Color>,
    /// Frame, rectangle, progress: corner radius in `u`, 0 to 960.
    #[serde(default)]
    pub radius: Option<f32>,
    /// Frame, rectangle, ellipse, line: the stroke (a line's colour and width).
    #[serde(default)]
    pub stroke: Option<Stroke>,
    /// Frame, rectangle, ellipse, text, image: drop shadows; need the `shadow` feature.
    #[serde(default)]
    pub shadows: Vec<Shadow>,
    /// Frame, group: whether children are clipped to it.
    #[serde(default)]
    pub clip: bool,
    /// Frame, group: its children, drawn in order.
    #[serde(default)]
    pub children: Vec<Node>,

    /// Text: the text (cleaned, up to 500 characters).
    #[serde(default)]
    pub text: Option<Text<{ limits::TEXT }>>,
    /// Text: font size in `u`, 6 to 200.
    #[serde(default)]
    pub size: Option<f32>,
    /// Text: weight, 100 to 900.
    #[serde(default)]
    pub weight: Option<u16>,
    /// Text: colour. Progress: the bar.
    #[serde(default)]
    pub color: Option<Color>,
    /// Text: alignment.
    #[serde(default)]
    pub align: Option<Align>,
    /// Text: most lines before it's cut, 1 to 10.
    #[serde(default)]
    pub max_lines: Option<u8>,
    /// Text: italic.
    #[serde(default)]
    pub italic: bool,

    /// Image: the PNG's name.
    #[serde(default)]
    pub asset: Option<AssetName>,
    /// Image: how it fills the box.
    #[serde(default)]
    pub fit: Option<Fit>,
    /// Image: a colour drawn over its opaque pixels.
    #[serde(default)]
    pub tint: Option<Color>,

    /// Progress: how full, 0 to 1.
    #[serde(default)]
    pub value: Option<f32>,

    /// Repeat: the list it draws, `group.list` (see [`crate::collections`]).
    #[serde(default)]
    pub list: Option<String>,
    /// Repeat: most items drawn, 1 to 50.
    #[serde(default)]
    pub max: Option<u8>,
    /// Repeat: which way items follow each other (default: a column).
    #[serde(default)]
    pub direction: Option<Direction>,
    /// Repeat: space between items in `u`, 0 to 200 (default 0).
    #[serde(default)]
    pub gap: Option<f32>,
    /// Repeat: each item's cell width in `u`, more than 0, at most the repeat's.
    #[serde(default)]
    pub item_w: Option<f32>,
    /// Repeat: each item's cell height in `u`, more than 0, at most the repeat's.
    #[serde(default)]
    pub item_h: Option<f32>,

    /// Properties set from expressions; the literal property above is the
    /// fallback while a binding has no value.
    #[serde(default)]
    pub bindings: Bindings,
}

/// A node's bound properties. Each is allowed where its literal property
/// is; `opacity` and `visible` on every kind.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Bindings {
    /// Text: the text.
    #[serde(default)]
    pub text: Option<Expr>,
    /// Text, progress: the colour.
    #[serde(default)]
    pub color: Option<Expr>,
    /// Frame, rectangle, ellipse, progress: the fill.
    #[serde(default)]
    pub fill: Option<Expr>,
    /// Image: the tint.
    #[serde(default)]
    pub tint: Option<Expr>,
    /// Progress: how full, 0 to 1 (clamped).
    #[serde(default)]
    pub value: Option<Expr>,
    /// Any kind: 0 to 1 (clamped).
    #[serde(default)]
    pub opacity: Option<Expr>,
    /// Any kind: whether it is drawn (fallback: yes).
    #[serde(default)]
    pub visible: Option<Expr>,
}

impl Bindings {
    /// Each bound property with its name and the type it wants.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Type, &Expr)> {
        [
            ("text", Type::Text, &self.text),
            ("color", Type::Color, &self.color),
            ("fill", Type::Color, &self.fill),
            ("tint", Type::Color, &self.tint),
            ("value", Type::Number, &self.value),
            ("opacity", Type::Number, &self.opacity),
            ("visible", Type::Bool, &self.visible),
        ]
        .into_iter()
        .filter_map(|(name, want, expr)| expr.as_ref().map(|expr| (name, want, expr)))
    }
}

const fn one() -> f32 {
    1.0
}

/// One face's look: a face, size class and state, and its root node.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Variant {
    /// The face it draws.
    pub face: Face,
    /// Its size class.
    #[serde(default)]
    pub size: SizeClass,
    /// Its state; `default` when absent.
    #[serde(default = "Name::default_state")]
    pub state: Name,
    /// The root, a frame.
    pub root: Node,
}

/// A panel's document.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Document {
    /// Its variants.
    pub variants: Vec<Variant>,
}

/// Renderer features (`meta.requires`) a document can need.
pub mod features {
    /// Drop shadows (not drawn by the software renderer).
    pub const SHADOW: &str = "shadow";
    /// Rotation (not drawn by the software renderer).
    pub const ROTATION: &str = "rotation";
}

/// The window, the largest any root may be.
const WINDOW: (f32, f32) = (1920.0, 1080.0);

/// A face's reference size, if it has a fixed one: a tile (and the phone
/// mirror of it) is 250×141 u, a page and the idle screen fill the window.
/// The other faces size to their content, up to the window.
fn reference_size(face: Face) -> Option<(f32, f32)> {
    match face {
        Face::Tile | Face::Phone => Some((250.0, 141.0)),
        Face::Page | Face::Ambient => Some(WINDOW),
        _ => None,
    }
}

/// Whether a root of `w`×`h` fits `face` at `size`: the reference size
/// exactly for `regular`, no larger for `compact`, no smaller for `large`;
/// always more than 0 and at most the window.
fn root_fits(face: Face, size: SizeClass, w: f32, h: f32) -> bool {
    let within = |(min_w, min_h): (f32, f32), (max_w, max_h): (f32, f32)| {
        w > 0.0 && h > 0.0 && (min_w..=max_w).contains(&w) && (min_h..=max_h).contains(&h)
    };
    match (reference_size(face), size) {
        (Some(reference), SizeClass::Regular) => w == reference.0 && h == reference.1,
        (Some(reference), SizeClass::Compact) => within((0.0, 0.0), reference),
        (Some(reference), SizeClass::Large) => within(reference, WINDOW),
        (None, _) => within((0.0, 0.0), WINDOW),
    }
}

/// Checks the document against itself and against `meta`.
pub(crate) fn check(document: &Document, meta: &Meta) -> Result<(), PackageError> {
    let declared: BTreeSet<&str> = meta.requires.iter().map(Name::as_str).collect();
    let sources: BTreeSet<&str> = meta.sources.iter().map(Name::as_str).collect();
    let knobs: BTreeMap<&str, Type> = meta
        .knobs
        .iter()
        .map(|k| (k.name.as_str(), k.value_type()))
        .collect();
    if document.variants.len() > limits::VARIANTS {
        return Err(PackageError::new(
            Code::TooMany,
            Position::field("document.variants"),
        ));
    }
    let mut keys = BTreeSet::new();
    for (variant_index, variant) in document.variants.iter().enumerate() {
        let at_variant = Position::Field {
            path: "document.variants",
            index: Some(variant_index),
        };
        if !meta.faces.contains(&variant.face) {
            return Err(PackageError::new(Code::FaceMismatch, at_variant));
        }
        if !keys.insert((variant.face, variant.size, variant.state.as_str())) {
            return Err(PackageError::new(Code::DuplicateVariant, at_variant));
        }
        let at_root = Position::Node {
            variant: variant_index,
            node: 0,
        };
        let root = &variant.root;
        if root.kind != NodeKind::Frame || root.x != 0.0 || root.y != 0.0 || root.rotation != 0.0 {
            return Err(PackageError::new(Code::BadRoot, at_root));
        }
        if !root_fits(variant.face, variant.size, root.w, root.h) {
            return Err(PackageError::new(Code::BadSize, at_root));
        }
        let mut walk = Walk {
            variant: variant_index,
            count: 0,
            shadows: 0,
            bindings: 0,
            ids: BTreeSet::new(),
            declared: &declared,
            scope: Scope::new(&sources, &knobs),
            drawn: 0,
            per_item: 1,
        };
        walk.node(&variant.root, 1)?;
    }
    for face in meta.faces.iter() {
        let has_default = keys.iter().any(|(f, size, state)| {
            f == face && *size == SizeClass::Regular && *state == Name::DEFAULT_STATE
        });
        if !has_default {
            return Err(PackageError::new(
                Code::FaceMismatch,
                Position::field("meta.faces"),
            ));
        }
    }
    Ok(())
}

/// Calls `f` with every node, its variant's index and its depth-first
/// index, in the order the validator counts them.
pub(crate) fn each_node<'a>(
    document: &'a Document,
    f: &mut dyn FnMut(usize, usize, &'a Node) -> Result<(), PackageError>,
) -> Result<(), PackageError> {
    fn walk<'a>(
        node: &'a Node,
        variant: usize,
        count: &mut usize,
        f: &mut dyn FnMut(usize, usize, &'a Node) -> Result<(), PackageError>,
    ) -> Result<(), PackageError> {
        let index = *count;
        *count += 1;
        f(variant, index, node)?;
        node.children
            .iter()
            .try_for_each(|child| walk(child, variant, count, f))
    }
    for (variant, v) in document.variants.iter().enumerate() {
        walk(&v.root, variant, &mut 0, f)?;
    }
    Ok(())
}

/// Calls `f` with every binding's position and expression, depth-first.
pub(crate) fn each_binding<'a>(
    document: &'a Document,
    f: &mut dyn FnMut(Position, &'a Expr) -> Result<(), PackageError>,
) -> Result<(), PackageError> {
    each_node(document, &mut |variant, node, n| {
        n.bindings.iter().try_for_each(|(property, _, expr)| {
            f(
                Position::Binding {
                    variant,
                    node,
                    property,
                },
                expr,
            )
        })
    })
}

/// A depth-first walk of one variant, counting against its limits.
struct Walk<'a> {
    variant: usize,
    count: usize,
    shadows: usize,
    bindings: usize,
    ids: BTreeSet<&'a str>,
    declared: &'a BTreeSet<&'a str>,
    /// What bindings may read here (a repeat's children also read its items).
    scope: Scope<'a>,
    /// Nodes drawn, counting a repeat's children once per item it may draw.
    drawn: usize,
    /// How many times the node being walked is drawn: a repeat's `max`
    /// inside one, else 1.
    per_item: usize,
}

impl<'a> Walk<'a> {
    fn node(&mut self, node: &'a Node, depth: usize) -> Result<(), PackageError> {
        let at = Position::Node {
            variant: self.variant,
            node: self.count,
        };
        let fail = |code| Err(PackageError::new(code, at));
        self.count += 1;
        // A repeat's children count once per item it may draw (§ 5.1).
        self.drawn += self.per_item;
        if self.drawn > limits::NODES {
            return fail(Code::TooManyNodes);
        }
        if depth > limits::DEPTH {
            return fail(Code::TooDeep);
        }
        if !self.ids.insert(node.id.as_str()) {
            return fail(Code::DuplicateId);
        }
        if let Err(code) = check_node(node) {
            return fail(code);
        }
        for (property, want, expr) in node.bindings.iter() {
            let at = Position::Binding {
                variant: self.variant,
                node: self.count - 1,
                property,
            };
            self.bindings += self.per_item;
            if self.bindings > limits::BINDINGS {
                return Err(PackageError::new(Code::TooManyBindings, at));
            }
            expr::check(expr, want, &self.scope).map_err(|code| PackageError::new(code, at))?;
        }
        self.shadows += node.shadows.len() * self.per_item;
        if self.shadows > limits::SHADOWS {
            return fail(Code::TooManyShadows);
        }
        let needs_shadow = !node.shadows.is_empty() && !self.declared.contains(features::SHADOW);
        let needs_rotation = node.rotation != 0.0 && !self.declared.contains(features::ROTATION);
        if needs_shadow || needs_rotation {
            return fail(Code::UndeclaredFeature);
        }
        if node.kind != NodeKind::Repeat {
            for child in &node.children {
                self.node(child, depth + 1)?;
            }
            return Ok(());
        }
        if self.scope.in_a_repeat() {
            return fail(Code::NestedRepeat);
        }
        let Some(max) = node.max else {
            return fail(Code::MissingProperty);
        };
        let list = node
            .list
            .as_deref()
            .and_then(|path| path.split_once('.'))
            .and_then(|(group, field)| expr::collection(group, field))
            .ok_or_else(|| PackageError::new(Code::UnknownSource, at))?;
        if !self.scope.contains_source(list.group) {
            return fail(Code::UndeclaredSource);
        }
        let (outside, outer_per_item) = (self.scope, self.per_item);
        self.scope = outside.in_repeat(list);
        self.per_item = outer_per_item * usize::from(max);
        for child in &node.children {
            self.node(child, depth + 1)?;
        }
        self.scope = outside;
        self.per_item = outer_per_item;
        Ok(())
    }
}

/// One node's own rules: its numbers in range and only its kind's properties.
fn check_node(node: &Node) -> Result<(), Code> {
    use NodeKind::{Ellipse, Frame, Group, Image, Line, Progress, Rectangle, Repeat, Text};
    let range = |ok: bool| if ok { Ok(()) } else { Err(Code::OutOfRange) };
    range(in_range(node.x, -1920.0..=3840.0) && in_range(node.y, -1080.0..=2160.0))?;
    range(in_range(node.w, 0.0..=1920.0) && in_range(node.h, 0.0..=1080.0))?;
    range(in_range(node.opacity, 0.0..=1.0) && in_range(node.rotation, -360.0..=360.0))?;
    range(node.radius.is_none_or(|r| in_range(r, 0.0..=960.0)))?;
    range(node.stroke.is_none_or(|s| in_range(s.width, 0.0..=20.0)))?;
    range(node.shadows.iter().all(|s| {
        in_range(s.x, -200.0..=200.0)
            && in_range(s.y, -200.0..=200.0)
            && in_range(s.blur, 0.0..=100.0)
    }))?;
    range(node.size.is_none_or(|s| in_range(s, 6.0..=200.0)))?;
    range(node.weight.is_none_or(|w| (100..=900).contains(&w)))?;
    range(node.max_lines.is_none_or(|m| (1..=10).contains(&m)))?;
    range(node.value.is_none_or(|v| in_range(v, 0.0..=1.0)))?;
    range(
        node.max
            .is_none_or(|m| (1..=limits::REPEAT).contains(&usize::from(m))),
    )?;
    range(node.gap.is_none_or(|g| in_range(g, 0.0..=200.0)))?;
    range(
        node.item_w
            .is_none_or(|w| w > 0.0 && in_range(w, 0.0..=node.w)),
    )?;
    range(
        node.item_h
            .is_none_or(|h| h > 0.0 && in_range(h, 0.0..=node.h)),
    )?;

    let k = node.kind;
    let b = &node.bindings;
    let allowed = |set: &[NodeKind]| set.contains(&k);
    let present: [(bool, &[NodeKind]); 15] = [
        (
            node.fill.is_some() || b.fill.is_some(),
            &[Frame, Rectangle, Ellipse, Progress],
        ),
        (node.radius.is_some(), &[Frame, Rectangle, Progress]),
        (node.stroke.is_some(), &[Frame, Rectangle, Ellipse, Line]),
        (
            !node.shadows.is_empty(),
            &[Frame, Rectangle, Ellipse, Text, Image],
        ),
        (node.clip, &[Frame, Group, Repeat]),
        (!node.children.is_empty(), &[Frame, Group, Repeat]),
        (node.text.is_some() || b.text.is_some(), &[Text]),
        (node.size.is_some(), &[Text]),
        (node.weight.is_some(), &[Text]),
        (node.color.is_some() || b.color.is_some(), &[Text, Progress]),
        (
            node.align.is_some() || node.max_lines.is_some() || node.italic,
            &[Text],
        ),
        (node.asset.is_some(), &[Image]),
        (
            node.fit.is_some() || node.tint.is_some() || b.tint.is_some(),
            &[Image],
        ),
        (node.value.is_some() || b.value.is_some(), &[Progress]),
        (
            node.list.is_some()
                || node.max.is_some()
                || node.direction.is_some()
                || node.gap.is_some()
                || node.item_w.is_some()
                || node.item_h.is_some(),
            &[Repeat],
        ),
    ];
    if present.iter().any(|(has, kinds)| *has && !allowed(kinds)) {
        return Err(Code::NotForKind);
    }
    let needs = match k {
        Text => node.text.is_some() && node.size.is_some(),
        Image => node.asset.is_some(),
        Progress => node.value.is_some(),
        Line => node.stroke.is_some(),
        Repeat => {
            node.list.is_some()
                && node.max.is_some()
                && node.item_w.is_some()
                && node.item_h.is_some()
        }
        Frame | Group | Rectangle | Ellipse => true,
    };
    if needs {
        Ok(())
    } else {
        Err(Code::MissingProperty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capabilities, Version, parse_package};

    /// A package with `faces` and `requires` and these `variants`.
    fn package(faces: &str, requires: &str, variants: &str) -> String {
        format!(
            r#"{{"meta": {{"format": 2, "min_launcher": "0.5.0", "requires": [{requires}], "id": "user.test",
               "kind": "panel", "version": "1.0.0", "default_language": "en", "name": {{"en": "Test"}},
               "faces": [{faces}]}},
              "document": {{"variants": [{variants}]}}}}"#
        )
    }

    /// A tile variant whose root frame has `children`.
    fn tile(children: &str) -> String {
        format!(
            r#"{{"face": "tile", "root": {{"id": "root", "type": "frame", "w": 250, "h": 141, "children": [{children}]}}}}"#
        )
    }

    fn check(faces: &str, requires: &str, variants: &str) -> Result<(), PackageError> {
        let can = Capabilities::new(Version::new(0, 5, 0), ["shadow", "rotation"]);
        parse_package(&package(faces, requires, variants), &can).map(|_| ())
    }

    fn code_of_child(child: &str) -> Option<Code> {
        check(r#""tile""#, "", &tile(child)).err().map(|e| e.code)
    }

    /// A tile (`shadow` declared, `apps` and `phones` sources) holding a
    /// repeat of `max` items with `extra` fields and `child`, then `after`.
    fn repeat_then(max: u32, extra: &str, child: &str, after: &str) -> Result<(), PackageError> {
        let repeat = format!(
            r#"{{"id": "apps", "type": "repeat", "w": 250, "h": 141, "list": "apps.list", "max": {max},
               "item_w": 250, "item_h": 20{extra}, "children": [{child}]}}{after}"#
        );
        let can = Capabilities::new(Version::new(0, 5, 0), ["shadow"]);
        let text = package(r#""tile""#, r#""shadow""#, &tile(&repeat)).replacen(
            r#""faces": ["tile"]"#,
            r#""faces": ["tile"], "sources": ["apps", "phones"]"#,
            1,
        );
        parse_package(&text, &can).map(|_| ())
    }

    fn repeat_tile(max: u32, extra: &str, child: &str) -> Result<(), PackageError> {
        repeat_then(max, extra, child, "")
    }

    fn rects(n: usize) -> String {
        (0..n)
            .map(|i| format!(r#"{{"id": "r{i}", "type": "rectangle", "w": 1, "h": 1}}"#))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// A repeat in a plain tile (no sources declared) with these fields.
    fn bare_repeat(fields: &str) -> Option<Code> {
        code_of_child(&format!(
            r#"{{"id": "r", "type": "repeat", "w": 10, "h": 10, {fields}}}"#
        ))
    }

    const FULL: &str = r#""list": "apps.list", "max": 1, "item_w": 10, "item_h": 10"#;

    const NAME: &str = r#"{"id": "name", "type": "text", "w": 250, "h": 20, "text": "…", "size": 14,
        "bindings": {"text": {"item": "name"}}}"#;

    const NESTED: &str = r#"{"id": "inner", "type": "repeat", "w": 10, "h": 10, "list": "apps.list",
        "max": 1, "item_w": 10, "item_h": 10}"#;

    #[test]
    fn a_repeat_draws_its_list_items_and_counts_them_all() {
        for extra in [
            "",
            r#", "direction": "row", "gap": 0, "clip": true"#,
            r#", "direction": "column", "gap": 200"#,
        ] {
            assert_eq!(repeat_tile(1, extra, NAME), Ok(()), "{extra}");
            assert_eq!(repeat_tile(50, extra, NAME), Ok(()), "{extra}");
        }
        assert_eq!(repeat_tile(5, "", ""), Ok(()));
        // Root + repeat + 2 × 249 = 500 nodes; one more child is 502.
        assert_eq!(repeat_tile(2, "", &rects(249)), Ok(()));
        assert_eq!(
            repeat_tile(2, "", &rects(250)).unwrap_err().code,
            Code::TooManyNodes
        );
        // Bindings count per item: 33 × 3 = 99, 34 × 3 = 102.
        let bound = r##"{"id": "b", "type": "rectangle", "w": 1, "h": 1,
            "bindings": {"opacity": 1, "visible": true, "fill": "#ffffff"}}"##;
        assert_eq!(repeat_tile(33, "", bound), Ok(()));
        assert_eq!(
            repeat_tile(34, "", bound).unwrap_err().code,
            Code::TooManyBindings
        );
        // Shadows per item: 4 × 1 is the cap.
        let shadowed = r##"{"id": "s", "type": "rectangle", "w": 1, "h": 1,
            "shadows": [{"blur": 2, "color": "#000000"}]}"##;
        assert_eq!(repeat_tile(4, "", shadowed), Ok(()));
        assert_eq!(
            repeat_tile(5, "", shadowed).unwrap_err().code,
            Code::TooManyShadows
        );
    }

    #[test]
    fn after_a_repeat_nodes_count_once_and_read_no_items() {
        let sibling = r#", {"id": "after", "type": "rectangle", "w": 1, "h": 1}"#;
        // 1 + 1 + 50 × 9 + 1 = 453: the sibling counts once.
        assert_eq!(repeat_then(50, "", &rects(9), sibling), Ok(()));
        let reads_item = r#", {"id": "after", "type": "text", "w": 1, "h": 1, "text": "x", "size": 10,
            "bindings": {"text": {"item": "name"}}}"#;
        let found = repeat_then(5, "", NAME, reads_item).unwrap_err();
        assert_eq!(
            (found.code, found.at),
            (
                Code::NotInRepeat,
                Position::Binding {
                    variant: 0,
                    node: 3,
                    property: "text"
                }
            )
        );
        let phones = r#", {"id": "p", "type": "repeat", "w": 10, "h": 10, "list": "phones.list",
            "max": 3, "item_w": 10, "item_h": 10, "children": [{"id": "n", "type": "text", "w": 10,
            "h": 10, "text": "x", "size": 10, "bindings": {"text": {"item": "name"}}}]}"#;
        assert_eq!(repeat_then(5, "", NAME, phones), Ok(()));
    }

    #[test]
    fn repeats_never_nest() {
        let wrapped =
            format!(r#"{{"id": "g", "type": "group", "w": 10, "h": 10, "children": [{NESTED}]}}"#);
        for (max, child) in [(5, NESTED), (1, NESTED), (1, wrapped.as_str())] {
            let found = repeat_tile(max, "", child).unwrap_err();
            assert_eq!(found.code, Code::NestedRepeat, "{max} {child}");
        }
        let found = repeat_tile(1, "", NESTED).unwrap_err();
        assert_eq!(
            found.at,
            Position::Node {
                variant: 0,
                node: 2
            }
        );
    }

    #[test]
    fn repeat_fields_have_their_rules() {
        let cases = [
            (repeat_tile(0, "", NAME), Code::OutOfRange),
            (repeat_tile(51, "", NAME), Code::OutOfRange),
            (repeat_tile(256, "", NAME), Code::OutOfRange),
            (
                repeat_tile(5, r#", "direction": "diagonal""#, NAME),
                Code::BadValue,
            ),
            (repeat_tile(5, r#", "radius": 3"#, NAME), Code::NotForKind),
            (
                repeat_tile(
                    5,
                    r#", "bindings": {"visible": {"call": "eq", "args": [{"item": "name"}, "x"]}}"#,
                    NAME,
                ),
                Code::NotInRepeat,
            ),
        ];
        for (found, want) in cases {
            assert_eq!(found.unwrap_err().code, want);
        }
        for missing in ["list", "max", "item_w", "item_h"] {
            let fields: Vec<&str> = FULL
                .split(", ")
                .filter(|f| !f.starts_with(&format!(r#""{missing}""#)))
                .collect();
            assert_eq!(
                bare_repeat(&fields.join(", ")),
                Some(Code::MissingProperty),
                "{missing}"
            );
        }
        let sized = r#""list": "apps.list", "max": 1"#;
        for bad in [
            r#""item_w": 0, "item_h": 10"#,
            r#""item_w": 10, "item_h": 0"#,
            r#""item_w": 11, "item_h": 10"#,
            r#""item_w": 10, "item_h": 11"#,
        ] {
            assert_eq!(
                bare_repeat(&format!("{sized}, {bad}")),
                Some(Code::OutOfRange),
                "{bad}"
            );
        }
        assert_eq!(
            bare_repeat(&format!(r#"{FULL}, "gap": -1"#)),
            Some(Code::OutOfRange)
        );
        assert_eq!(
            bare_repeat(&format!(r#"{FULL}, "gap": 201"#)),
            Some(Code::OutOfRange)
        );
        assert_eq!(bare_repeat(FULL), Some(Code::UndeclaredSource));
        for list in ["clock.now", "apps", "apps.list.x", "apps.grid"] {
            let fields = FULL.replace("apps.list", list);
            assert_eq!(bare_repeat(&fields), Some(Code::UnknownSource), "{list}");
        }
        for field in [
            r#""list": "apps.list""#,
            r#""max": 3"#,
            r#""direction": "row""#,
            r#""gap": 1"#,
            r#""item_w": 1"#,
            r#""item_h": 1"#,
        ] {
            let child = format!(r#"{{"id": "f", "type": "frame", "w": 10, "h": 10, {field}}}"#);
            assert_eq!(code_of_child(&child), Some(Code::NotForKind), "{field}");
        }
        assert_eq!(code_of_child(NAME), Some(Code::NotInRepeat));
    }

    #[test]
    fn every_node_kind_parses_with_its_own_properties() {
        let children = [
            r##"{"id": "r", "type": "rectangle", "w": 10, "h": 10, "fill": "#ff0000", "radius": 2, "stroke": {"color": "#000000", "width": 1}}"##,
            r##"{"id": "e", "type": "ellipse", "w": 10, "h": 10, "fill": "#00ff00"}"##,
            r##"{"id": "l", "type": "line", "w": 10, "h": 0, "stroke": {"color": "#ffffff", "width": 2}}"##,
            r##"{"id": "t", "type": "text", "w": 100, "h": 20, "text": "Hi", "size": 14, "align": "end", "max_lines": 2, "italic": true, "label": "Greeting"}"##,
            r#"{"id": "i", "type": "image", "w": 48, "h": 48, "asset": "logo", "fit": "contain"}"#,
            r##"{"id": "p", "type": "progress", "w": 200, "h": 8, "value": 0.5, "fill": "#333333", "color": "#4caf50", "radius": 4}"##,
        ];
        assert_eq!(check(r#""tile""#, "", &tile(&children.join(","))), Ok(()));
    }

    #[test]
    fn a_property_of_another_kind_is_refused() {
        for child in [
            r#"{"id": "t", "type": "text", "w": 1, "h": 1, "text": "a", "size": 10, "asset": "logo"}"#,
            r##"{"id": "r", "type": "rectangle", "w": 1, "h": 1, "color": "#ffffff"}"##,
            r#"{"id": "e", "type": "ellipse", "w": 1, "h": 1, "clip": true}"#,
            r#"{"id": "i", "type": "image", "w": 1, "h": 1, "asset": "a", "children": [{"id": "x", "type": "ellipse", "w": 1, "h": 1}]}"#,
        ] {
            assert_eq!(code_of_child(child), Some(Code::NotForKind), "{child}");
        }
    }

    #[test]
    fn a_kind_without_what_it_needs_is_refused() {
        for child in [
            r#"{"id": "t", "type": "text", "w": 1, "h": 1, "text": "a"}"#,
            r#"{"id": "i", "type": "image", "w": 1, "h": 1}"#,
            r#"{"id": "p", "type": "progress", "w": 1, "h": 1}"#,
            r#"{"id": "l", "type": "line", "w": 1, "h": 1}"#,
        ] {
            assert_eq!(code_of_child(child), Some(Code::MissingProperty), "{child}");
        }
    }

    #[test]
    fn numbers_outside_their_range_are_refused() {
        for child in [
            r#"{"id": "a", "type": "ellipse", "w": 1921, "h": 1}"#,
            r#"{"id": "a", "type": "ellipse", "w": 1, "h": 1, "opacity": 1.5}"#,
            r#"{"id": "a", "type": "ellipse", "w": 1, "h": 1, "x": 1e30}"#,
            r#"{"id": "a", "type": "text", "w": 1, "h": 1, "text": "a", "size": 2}"#,
            r#"{"id": "a", "type": "text", "w": 1, "h": 1, "text": "a", "size": 10, "weight": 950}"#,
            r#"{"id": "a", "type": "progress", "w": 1, "h": 1, "value": -0.1}"#,
            r#"{"id": "a", "type": "rectangle", "w": 1, "h": 1, "rotation": 720}"#,
        ] {
            assert_eq!(code_of_child(child), Some(Code::OutOfRange), "{child}");
        }
    }

    #[test]
    fn ids_must_be_unique_in_a_variant() {
        let twice = r#"{"id": "a", "type": "ellipse", "w": 1, "h": 1}, {"id": "a", "type": "ellipse", "w": 1, "h": 1}"#;
        let found = check(r#""tile""#, "", &tile(twice)).unwrap_err();
        assert_eq!(
            (found.code, found.at),
            (
                Code::DuplicateId,
                Position::Node {
                    variant: 0,
                    node: 2
                }
            )
        );
    }

    #[test]
    fn node_count_depth_and_shadows_are_capped() {
        let many = (0..limits::NODES)
            .map(|i| format!(r#"{{"id": "n{i}", "type": "ellipse", "w": 1, "h": 1}}"#));
        assert_eq!(
            code_of_child(&many.collect::<Vec<_>>().join(",")),
            Some(Code::TooManyNodes)
        );

        let mut deep = r#"{"id": "leaf", "type": "ellipse", "w": 1, "h": 1}"#.to_owned();
        for level in 0..limits::DEPTH - 1 {
            deep = format!(
                r#"{{"id": "f{level}", "type": "frame", "w": 1, "h": 1, "children": [{deep}]}}"#
            );
        }
        assert_eq!(code_of_child(&deep), Some(Code::TooDeep));

        let shadow = r##"{"blur": 4, "color": "#00000080"}"##;
        let shadowed = format!(
            r#"{{"id": "s", "type": "rectangle", "w": 1, "h": 1, "shadows": [{}]}}"#,
            [shadow; 5].join(",")
        );
        assert_eq!(
            check(r#""tile""#, r#""shadow""#, &tile(&shadowed))
                .unwrap_err()
                .code,
            Code::TooManyShadows
        );
    }

    #[test]
    fn shadows_and_rotation_must_be_declared_in_requires() {
        let shadowed = r##"{"id": "s", "type": "rectangle", "w": 1, "h": 1, "shadows": [{"blur": 4, "color": "#000000"}]}"##;
        assert_eq!(code_of_child(shadowed), Some(Code::UndeclaredFeature));
        assert_eq!(check(r#""tile""#, r#""shadow""#, &tile(shadowed)), Ok(()));
        let rotated = r#"{"id": "r", "type": "rectangle", "w": 1, "h": 1, "rotation": 45}"#;
        assert_eq!(code_of_child(rotated), Some(Code::UndeclaredFeature));
        assert_eq!(check(r#""tile""#, r#""rotation""#, &tile(rotated)), Ok(()));
    }

    #[test]
    fn variants_match_the_declared_faces() {
        let page =
            r#"{"face": "page", "root": {"id": "root", "type": "frame", "w": 1920, "h": 1080}}"#;
        let code = |faces, variants: &str| check(faces, "", variants).err().map(|e| e.code);
        assert_eq!(
            code(r#""tile""#, &format!("{},{page}", tile(""))),
            Some(Code::FaceMismatch)
        );
        assert_eq!(
            code(r#""tile", "page""#, &tile("")),
            Some(Code::FaceMismatch)
        );
        assert_eq!(
            code(r#""tile""#, &format!("{0},{0}", tile(""))),
            Some(Code::DuplicateVariant)
        );
        let compact = tile("").replace(
            r#""face": "tile","#,
            r#""face": "tile", "size": "compact", "state": "attention","#,
        );
        assert_eq!(code(r#""tile""#, &format!("{},{compact}", tile(""))), None);
        assert_eq!(code(r#""tile""#, &compact), Some(Code::FaceMismatch));
    }

    #[test]
    fn a_root_is_an_unrotated_frame_at_the_origin() {
        let code = |variant: String| {
            check(r#""tile""#, r#""rotation""#, &variant)
                .err()
                .map(|e| e.code)
        };
        for (from, to) in [
            (r#""type": "frame""#, r#""type": "group""#),
            (r#""type": "frame""#, r#""type": "rectangle""#),
            (r#""id": "root","#, r#""id": "root", "x": 1,"#),
            (r#""id": "root","#, r#""id": "root", "y": -1,"#),
            (r#""id": "root","#, r#""id": "root", "rotation": 45,"#),
        ] {
            assert_eq!(
                code(tile("").replace(from, to)),
                Some(Code::BadRoot),
                "{to}"
            );
        }
    }

    #[test]
    fn a_root_fits_its_face_and_size_class() {
        let root = |face: &str, size: &str, w: u32, h: u32| {
            let variant = format!(
                r#"{{"face": "{face}", "size": "{size}", "root": {{"id": "root", "type": "frame", "w": {w}, "h": {h}}}}}"#
            );
            let regular = |w, h| {
                format!(
                    r#"{{"face": "{face}", "root": {{"id": "root", "type": "frame", "w": {w}, "h": {h}}}}}"#
                )
            };
            let base = match face {
                "tile" | "phone" => regular(250, 141),
                "page" | "ambient" => regular(1920, 1080),
                _ => regular(48, 48),
            };
            let variants = if size == "regular" {
                variant
            } else {
                format!("{base},{variant}")
            };
            check(&format!(r#""{face}""#), "", &variants)
                .err()
                .map(|e| e.code)
        };
        assert_eq!(root("tile", "regular", 250, 141), None);
        assert_eq!(root("tile", "regular", 260, 141), Some(Code::BadSize));
        assert_eq!(root("phone", "regular", 250, 140), Some(Code::BadSize));
        assert_eq!(root("page", "regular", 1920, 1080), None);
        assert_eq!(root("page", "regular", 1280, 720), Some(Code::BadSize));
        assert_eq!(root("ambient", "regular", 1920, 1000), Some(Code::BadSize));
        assert_eq!(root("tile", "compact", 200, 120), None);
        assert_eq!(root("tile", "compact", 260, 120), Some(Code::BadSize));
        assert_eq!(root("tile", "compact", 0, 120), Some(Code::BadSize));
        assert_eq!(root("tile", "large", 300, 170), None);
        assert_eq!(root("tile", "large", 200, 170), Some(Code::BadSize));
        assert_eq!(root("corner", "regular", 48, 48), None);
        assert_eq!(root("corner", "regular", 1920, 1080), None);
        assert_eq!(root("corner", "regular", 0, 0), Some(Code::BadSize));
        assert_eq!(root("banner", "regular", 1921, 100), Some(Code::BadSize));
    }

    #[test]
    fn limits_pass_at_their_edge() {
        let many = (1..limits::NODES)
            .map(|i| format!(r#"{{"id": "n{i}", "type": "ellipse", "w": 1, "h": 1}}"#));
        assert_eq!(code_of_child(&many.collect::<Vec<_>>().join(",")), None);

        let mut deep = r#"{"id": "leaf", "type": "ellipse", "w": 1, "h": 1}"#.to_owned();
        for level in 0..limits::DEPTH - 2 {
            deep = format!(
                r#"{{"id": "g{level}", "type": "group", "w": 1, "h": 1, "children": [{deep}]}}"#
            );
        }
        assert_eq!(code_of_child(&deep), None);

        let shadowed = |id: &str, n: usize| {
            let shadow = r##"{"blur": 4, "color": "#00000080"}"##;
            format!(
                r#"{{"id": "{id}", "type": "rectangle", "w": 1, "h": 1, "shadows": [{}]}}"#,
                vec![shadow; n].join(",")
            )
        };
        let shadows = |children: &str| {
            check(r#""tile""#, r#""shadow""#, &tile(children))
                .err()
                .map(|e| e.code)
        };
        assert_eq!(shadows(&shadowed("a", 4)), None);
        assert_eq!(
            shadows(&format!("{},{}", shadowed("a", 2), shadowed("b", 2))),
            None
        );
        assert_eq!(
            shadows(&format!("{},{}", shadowed("a", 2), shadowed("b", 3))),
            Some(Code::TooManyShadows)
        );
    }

    #[test]
    fn variants_are_capped() {
        let states = (0..limits::VARIANTS).map(|i| {
            tile("").replace(
                r#""face": "tile","#,
                &format!(r#""face": "tile", "state": "s{i}","#),
            )
        });
        let variants = std::iter::once(tile(""))
            .chain(states)
            .collect::<Vec<_>>()
            .join(",");
        let found = check(r#""tile""#, "", &variants).unwrap_err();
        assert_eq!(
            (found.code, found.at),
            (Code::TooMany, Position::field("document.variants"))
        );
    }

    #[test]
    fn integers_too_big_for_their_field_are_out_of_range() {
        for child in [
            r#"{"id": "a", "type": "text", "w": 1, "h": 1, "text": "a", "size": 10, "weight": 70000}"#,
            r#"{"id": "a", "type": "text", "w": 1, "h": 1, "text": "a", "size": 10, "max_lines": 300}"#,
            r#"{"id": "a", "type": "text", "w": 1, "h": 1, "text": "a", "size": 10, "max_lines": -1}"#,
        ] {
            assert_eq!(code_of_child(child), Some(Code::OutOfRange), "{child}");
        }
    }

    #[test]
    fn more_ranges_are_checked() {
        for child in [
            r##"{"id": "a", "type": "rectangle", "w": 1, "h": 1, "radius": 961}"##,
            r##"{"id": "a", "type": "line", "w": 1, "h": 1, "stroke": {"color": "#000000", "width": 21}}"##,
            r##"{"id": "a", "type": "ellipse", "w": 1, "h": 1081}"##,
        ] {
            assert_eq!(code_of_child(child), Some(Code::OutOfRange), "{child}");
        }
        for shadow in [
            r##"{"x": 201, "blur": 1, "color": "#000000"}"##,
            r##"{"y": -201, "blur": 1, "color": "#000000"}"##,
            r##"{"blur": 101, "color": "#000000"}"##,
        ] {
            let child = format!(
                r#"{{"id": "a", "type": "rectangle", "w": 1, "h": 1, "shadows": [{shadow}]}}"#
            );
            assert_eq!(
                check(r#""tile""#, r#""shadow""#, &tile(&child))
                    .unwrap_err()
                    .code,
                Code::OutOfRange,
                "{shadow}"
            );
        }
        assert_eq!(
            code_of_child(r#"{"id": "a", "type": "ellipse", "w": 1920, "h": 1080}"#),
            None
        );
    }

    #[test]
    fn a_group_holds_children_but_paints_nothing() {
        let group = r#"{"id": "g", "type": "group", "w": 10, "h": 10, "clip": true, "children": [{"id": "e", "type": "ellipse", "w": 1, "h": 1}]}"#;
        assert_eq!(code_of_child(group), None);
        assert_eq!(
            code_of_child(r##"{"id": "g", "type": "group", "w": 1, "h": 1, "fill": "#000000"}"##),
            Some(Code::NotForKind)
        );
    }

    #[test]
    fn a_document_written_as_arrays_is_refused() {
        let positional = package(r#""tile""#, "", "").replace(
            r#""document": {"variants": []}"#,
            r#""document": [[["tile", "regular", "default", ["root", "frame", 0, 0, 250, 141]]]]"#,
        );
        let can = Capabilities::new(Version::new(0, 5, 0), ["shadow"]);
        assert_eq!(
            parse_package(&positional, &can).unwrap_err().code,
            Code::WrongType
        );
        assert_eq!(
            code_of_child(r#"["a", "ellipse", 0, 0, 1, 1]"#),
            Some(Code::WrongType)
        );
    }

    #[test]
    fn a_node_position_reads_as_variant_and_node() {
        let at = Position::Node {
            variant: 2,
            node: 7,
        };
        assert_eq!(at.to_string(), "document.variants[2] node 7");
        assert_eq!(AssetName::parse("Logo"), Err(Code::BadAssetName));
    }

    fn bound(child: &str) -> Result<(), PackageError> {
        let text = package(r#""tile""#, "", &tile(child)).replace(
            r#""faces""#,
            r#""sources": ["network", "history"], "faces""#,
        );
        let can = Capabilities::new(Version::new(0, 5, 0), ["shadow"]);
        parse_package(&text, &can).map(|_| ())
    }

    #[test]
    fn bindings_are_checked_with_their_node() {
        let ok = r##"{"id": "t", "type": "text", "w": 100, "h": 20, "text": "…", "size": 14,
            "bindings": {"text": {"call": "text", "args": [{"source": "history.waiting"}]},
                         "color": {"if": [{"source": "network.connected"}, "#ffffff", "#888888"]},
                         "visible": {"call": "gt", "args": [{"source": "history.waiting"}, 0]}}}"##;
        assert_eq!(bound(ok), Ok(()));
        let wrong_kind =
            r#"{"id": "e", "type": "ellipse", "w": 1, "h": 1, "bindings": {"text": "x"}}"#;
        assert_eq!(bound(wrong_kind).unwrap_err().code, Code::NotForKind);
        let undeclared = r#"{"id": "e", "type": "ellipse", "w": 1, "h": 1, "bindings": {"visible": {"source": "clock.hour24"}}}"#;
        let error = bound(undeclared).unwrap_err();
        assert_eq!(
            (error.code, error.at),
            (
                Code::UndeclaredSource,
                Position::Binding {
                    variant: 0,
                    node: 1,
                    property: "visible"
                }
            )
        );
        assert_eq!(
            error.to_string(),
            "UNDECLARED_SOURCE at document.variants[0] node 1 bindings.visible"
        );
        let wrong_type =
            r#"{"id": "e", "type": "ellipse", "w": 1, "h": 1, "bindings": {"opacity": true}}"#;
        assert_eq!(bound(wrong_type).unwrap_err().code, Code::TypeMismatch);
        let unknown = r#"{"id": "e", "type": "ellipse", "w": 1, "h": 1, "bindings": {"size": 3}}"#;
        assert_eq!(bound(unknown).unwrap_err().code, Code::UnknownField);
    }

    #[test]
    fn bindings_are_capped_per_variant() {
        let node = |i| {
            format!(
                r#"{{"id": "n{i}", "type": "ellipse", "w": 1, "h": 1, "bindings": {{"opacity": 1, "visible": true}}}}"#
            )
        };
        let half = limits::BINDINGS / 2;
        assert_eq!(
            bound(&(0..half).map(node).collect::<Vec<_>>().join(",")),
            Ok(())
        );
        assert_eq!(
            bound(&(0..=half).map(node).collect::<Vec<_>>().join(","))
                .unwrap_err()
                .code,
            Code::TooManyBindings
        );
    }

    #[test]
    fn meta_sources_must_be_known_groups() {
        let text = package(r#""tile""#, "", &tile(""))
            .replace(r#""faces""#, r#""sources": ["clock", "ssid"], "faces""#);
        let can = Capabilities::new(Version::new(0, 5, 0), ["shadow"]);
        let error = parse_package(&text, &can).unwrap_err();
        assert_eq!(
            (error.code, error.at),
            (
                Code::UnknownSource,
                Position::Field {
                    path: "meta.sources",
                    index: Some(1)
                }
            )
        );
    }

    #[test]
    fn unknown_kinds_and_properties_are_refused() {
        assert_eq!(
            code_of_child(r#"{"id": "q", "type": "qrcode", "w": 1, "h": 1}"#),
            Some(Code::BadValue)
        );
        assert_eq!(
            code_of_child(r#"{"id": "q", "type": "ellipse", "w": 1, "h": 1, "blend": "multiply"}"#),
            Some(Code::UnknownField)
        );
    }

    #[test]
    fn colours_are_six_or_eight_hex_digits() {
        assert_eq!(
            Color::parse("#1e2633"),
            Ok(Color {
                r: 0x1e,
                g: 0x26,
                b: 0x33,
                a: 255
            })
        );
        assert_eq!(Color::parse("#ffffff40").map(|c| c.a), Ok(0x40));
        for bad in [
            "1e2633",
            "#1e263",
            "#1e26331",
            "#gggggg",
            "#+1e263",
            "red",
            "#１２３４５６",
        ] {
            assert_eq!(Color::parse(bad), Err(Code::BadColor), "{bad}");
        }
    }

    #[test]
    fn node_ids_and_asset_names_are_short_identifiers() {
        assert!(NodeId::parse("title_2").is_ok());
        assert!(NodeId::parse(&"a".repeat(33)).is_err());
        assert!(AssetName::parse(&"a".repeat(40)).is_ok());
        for bad in ["", "Title", "a b", "a.png", "../x"] {
            assert_eq!(NodeId::parse(bad), Err(Code::BadNodeId), "{bad}");
        }
    }
}
