//! Pointer tools. The UI (or an agent through the control channel) feeds [`PointerEvent`]s in page
//! coordinates; tools turn gestures into document edits through interactions (one undo step per
//! gesture) and publish overlay hints (marquee, guides, size label, cursor) for the canvas.

use std::collections::BTreeMap;

use figcraft_color::{ImageScaleMode, PaintKind};
use figcraft_doc::geometry::select_target;
use figcraft_doc::{Document, LayoutMode, Node, NodeId, NodeKind, Sizing, StrokeAlign, StrokeCap, TextAutoResize, TextData, TextPath, VectorNetwork};
use figcraft_geom::{Affine, Point, Rect, Vec2};
use serde::Serialize;
use serde_json::json;

use crate::arc_handles::{ArcHandle, ArcHint};
use crate::cmd::arrange::move_world;
use crate::cmd::create::frame_at;
use crate::cmd::edit::roots;
use crate::shape_handles::{ShapeHandle, ShapeHint};
use crate::snap::{Guide, Targets};
use crate::{Result, Session, textedit};

mod click;
mod drag;
mod draw;
mod handles;
mod keys;
mod pen;
mod pointer;

use handles::DRAG_SLOP_PX;
/// 顶层画框的「本体」：文档里它和画布一样（按下是框选），只有标题文字能选中它。
///
/// 自动布局的画框例外——它的本体就是设计面，点本体直接选中。
pub(crate) fn frame_background(doc: &Document, page: NodeId, leaf: NodeId) -> bool {
    doc.node(leaf).is_some_and(|n| {
        n.parent == Some(page)
            && n.kind.is_frame_like()
            && !n.children.is_empty()
            && n.layout.is_none()
            && !crate::widgets::is_widget(n)
            && !n.is_board_atom()
    })
}

#[cfg(test)]
pub(crate) use handles::scale_factor;
pub use handles::{
    PENCIL_STROKE, SMALL_BOX_PX, anchor_name, anchor_of, box_sides_px, handle_at, handle_at_corners, label_at, locked_in, scale_anchor_point,
    selection_box, selection_frame,
};
pub(crate) use handles::{SNAP_PX, handle_cursor, readable_text_fills, resized_box};
#[cfg(test)]
pub(crate) use pen::pressure_profile;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub enum ToolId {
    #[default]
    Move,
    Scale,
    Hand,
    Frame,
    Section,
    Slice,
    Rectangle,
    Line,
    Arrow,
    Ellipse,
    Polygon,
    Star,
    Pen,
    Pencil,
    Text,
    TextOnPath,
    Comment,
    Eyedropper,
    ShapeBuilder,
    // Board tools.
    Sticky,
    BoardShape,
    Connector,
    Marker,
    Highlighter,
    Eraser,
    Stamp,
}

impl ToolId {
    pub const ALL: [ToolId; 26] = [
        ToolId::Move,
        ToolId::Scale,
        ToolId::Hand,
        ToolId::Frame,
        ToolId::Section,
        ToolId::Slice,
        ToolId::Rectangle,
        ToolId::Line,
        ToolId::Arrow,
        ToolId::Ellipse,
        ToolId::Polygon,
        ToolId::Star,
        ToolId::Pen,
        ToolId::Pencil,
        ToolId::Text,
        ToolId::TextOnPath,
        ToolId::Comment,
        ToolId::Eyedropper,
        ToolId::ShapeBuilder,
        ToolId::Sticky,
        ToolId::BoardShape,
        ToolId::Connector,
        ToolId::Marker,
        ToolId::Highlighter,
        ToolId::Eraser,
        ToolId::Stamp,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ToolId::Move => "move",
            ToolId::Scale => "scale",
            ToolId::Hand => "hand",
            ToolId::Frame => "frame",
            ToolId::Section => "section",
            ToolId::Slice => "slice",
            ToolId::Rectangle => "rectangle",
            ToolId::Line => "line",
            ToolId::Arrow => "arrow",
            ToolId::Ellipse => "ellipse",
            ToolId::Polygon => "polygon",
            ToolId::Star => "star",
            ToolId::Pen => "pen",
            ToolId::Pencil => "pencil",
            ToolId::Text => "text",
            ToolId::TextOnPath => "textOnPath",
            ToolId::Comment => "comment",
            ToolId::Eyedropper => "eyedropper",
            ToolId::ShapeBuilder => "shapeBuilder",
            ToolId::Sticky => "sticky",
            ToolId::BoardShape => "shape",
            ToolId::Connector => "connector",
            ToolId::Marker => "marker",
            ToolId::Highlighter => "highlighter",
            ToolId::Eraser => "eraser",
            ToolId::Stamp => "stamp",
        }
    }
    pub fn from_id(s: &str) -> Option<ToolId> {
        ToolId::ALL.into_iter().find(|t| t.id().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            ToolId::Move => "Move",
            ToolId::Scale => "Scale",
            ToolId::Hand => "Hand tool",
            ToolId::Frame => "Frame",
            ToolId::Section => "Section",
            ToolId::Slice => "Slice",
            ToolId::Rectangle => "Rectangle",
            ToolId::Line => "Line",
            ToolId::Arrow => "Arrow",
            ToolId::Ellipse => "Ellipse",
            ToolId::Polygon => "Polygon",
            ToolId::Star => "Star",
            ToolId::Pen => "Pen",
            ToolId::Pencil => "Pencil",
            ToolId::Text => "Text",
            ToolId::TextOnPath => "Text on a path",
            ToolId::Comment => "Comment",
            ToolId::Eyedropper => "Pick color",
            ToolId::ShapeBuilder => "Shape builder",
            ToolId::Sticky => "Sticky note",
            ToolId::BoardShape => "Shapes",
            ToolId::Connector => "Connector",
            ToolId::Marker => "Marker",
            ToolId::Highlighter => "Highlighter",
            ToolId::Eraser => "Eraser",
            ToolId::Stamp => "Stamp",
        }
    }

    /// The tools of a board's toolbar (a design file's toolbar has the others).
    pub fn on_boards(self) -> bool {
        matches!(
            self,
            ToolId::Sticky | ToolId::BoardShape | ToolId::Connector | ToolId::Marker | ToolId::Highlighter | ToolId::Eraser | ToolId::Stamp
        )
    }
    /// The tool's key (from [`crate::keymap`]; empty when it has none).
    pub fn shortcut(self) -> &'static str {
        let id = self.id();
        crate::keymap::KEYS
            .iter()
            .find(|b| if self == ToolId::Eyedropper { b.id == "edit.pickColor" } else { b.id.strip_prefix("tool.") == Some(id) })
            .and_then(|b| b.keys(crate::keymap::Platform::current()).first().copied())
            .unwrap_or("")
    }

    /// The size a click (or Enter, `tool.place`) makes with a shape or frame tool.
    pub fn click_size(self) -> (f64, f64) {
        match self {
            ToolId::Line | ToolId::Arrow => (100.0, 0.0),
            ToolId::Section => (400.0, 300.0),
            _ => (100.0, 100.0),
        }
    }

    pub fn draws_shape(self) -> bool {
        matches!(
            self,
            ToolId::Frame
                | ToolId::Section
                | ToolId::Slice
                | ToolId::Rectangle
                | ToolId::Ellipse
                | ToolId::Polygon
                | ToolId::Star
                | ToolId::Line
                | ToolId::Arrow
                | ToolId::BoardShape
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PointerKind {
    Down,
    Move,
    Up,
    /// Pointer moved without a button.
    Hover,
    DoubleClick,
}

/// A pointer event in page coordinates (and screen coordinates for panning).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub x: f64,
    pub y: f64,
    /// Screen position (logical pixels).
    pub sx: f64,
    pub sy: f64,
    pub shift: bool,
    pub alt: bool,
    /// Cmd on macOS, Ctrl elsewhere.
    pub cmd: bool,
    pub ctrl: bool,
    /// Screen pixels per page unit.
    pub zoom: f64,
    /// Pen pressure, 0..1, where the device reports it (`None`: a mouse, or a platform that doesn't).
    pub pressure: Option<f32>,
}

/// The pointer is still within [`DRAG_SLOP_PX`] of where it was pressed.
pub(crate) fn within_slop(ev: &PointerEvent, start: Point) -> bool {
    (ev.p() - start).hypot() * ev.zoom.max(1e-6) < DRAG_SLOP_PX
}

impl PointerEvent {
    pub fn at(kind: PointerKind, x: f64, y: f64, zoom: f64) -> Self {
        Self { kind, x, y, sx: x * zoom, sy: y * zoom, shift: false, alt: false, cmd: false, ctrl: false, zoom, pressure: None }
    }
    pub(crate) fn p(&self) -> Point {
        Point::new(self.x, self.y)
    }
    fn tol(&self) -> f64 {
        1.0 / self.zoom.max(1e-6)
    }
}

/// Cursor hint for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Default)]
pub enum Cursor {
    #[default]
    Default,
    Crosshair,
    Move,
    /// Resize along an angle (degrees, 0 = horizontal).
    Resize(f64),
    /// Rotate, turned to the corner it is near: the direction from the box's centre to the corner
    /// (while rotating: to the pointer), in degrees, 0 = right and 90 = down.
    Rotate(f64),
    Grab,
    Grabbing,
    Text,
    Pen(PenCursor),
    /// A drag here makes a copy (Alt).
    Copy,
    /// Over a clickable part of a widget.
    Pointer,
    Eyedropper,
    Comment,
    /// What the gesture is doing will be undone when it ends (an edit only a main component can
    /// make).
    NotAllowed,
}

/// What a click with the pen would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub enum PenCursor {
    /// A new point.
    #[default]
    New,
    /// On a point: closes the path, connects to the point or goes on from it.
    Close,
    /// On a segment: adds a point there.
    Add,
    /// Bends the segment or the point's handles (the Bend tool; Cmd/Ctrl in vector edit mode).
    Bend,
}

impl ToolId {
    /// The tool's cursor over empty canvas.
    pub fn cursor(self) -> Cursor {
        match self {
            ToolId::Hand => Cursor::Grab,
            ToolId::Pen => Cursor::Pen(PenCursor::New),
            ToolId::Comment => Cursor::Comment,
            ToolId::Eyedropper => Cursor::Eyedropper,
            // Text on a path acts only over a path, where it shows the I-beam.
            ToolId::Move | ToolId::Scale | ToolId::TextOnPath => Cursor::Default,
            // The other tools make something where they are pressed: shapes, frames, slices, pencil
            // strokes, a new text (over a text the Text tool shows the I-beam instead).
            _ => Cursor::Crosshair,
        }
    }
}

/// Selection handle kinds: corners (0 tl, 1 tr, 2 br, 3 bl), edges (0 t, 1 r, 2 b, 3 l) and the
/// rotation zones outside the corners.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Handle {
    Corner(u8),
    Edge(u8),
    Rotate(u8),
}

/// What the canvas should draw on top of the scene.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    pub marquee: Option<[f64; 4]>,
    pub guides: Vec<Guide>,
    pub cursor: Cursor,
    /// Pen tool: the in-progress path's anchor points and the rubber band to the pointer.
    pub pen: Vec<[f64; 2]>,
    /// Pencil: the colour of the stroke in progress, for its preview (white over a dark page or
    /// frame); None outside a Pencil stroke.
    pub pen_color: Option<figcraft_color::Color>,
    pub rubber: Option<[f64; 4]>,
    /// Pen tool: the handles shaping the segments at the pen (anchor x, y, handle x, y): the last
    /// anchor's in and out handles and the one before's out handle.
    pub pen_handles: Vec<[f64; 4]>,
    /// Pen tool: where the last anchor's out handle ends, when it has one: the rubber band is the
    /// curve a click would make, leaving along it.
    pub rubber_handle: Option<[f64; 2]>,
    /// Frame/section the pointer is drawing or dropping into (highlighted).
    pub drop_target: Option<NodeId>,
    /// Boards: the object a stamp would stick to, under the stamp tool's pointer or a stamp being
    /// dragged (highlighted).
    pub stamp_target: Option<NodeId>,
    /// Connector tool: the attach points (side middles, page x, y) of the object under the pointer,
    /// and whether the connector end is at each.
    pub attach: Vec<([f64; 2], bool)>,
    /// Auto layout insertion indicator (x0, y0, x1, y1).
    pub insert_line: Option<[f64; 4]>,
    /// The instance an Alt-drop of a component from the Assets panel would swap (highlighted).
    pub swap_target: Option<NodeId>,
    /// Auto layout grid cell a drag would drop into (x0, y0, x1, y1).
    pub drop_cell: Option<[f64; 4]>,
    /// Vector edit mode: what the pointer is over, the lasso being drawn and the closed area the
    /// paint bucket would fill (page-space polygons).
    pub vector_hover: Option<crate::vectoredit::VHit>,
    /// The edited layer `vector_hover` is on (several layers can be in edit mode).
    pub vector_hover_on: Option<NodeId>,
    /// Vector edit mode: the point a press on the outline would add (the pen's ghost anchor where
    /// it is over a segment, or the midpoint of the segment under the Move tool).
    pub vector_add: Option<VectorAdd>,
    /// Vector edit mode, Variable width tool: the width point, handle or path under the pointer.
    pub width_hover: Option<crate::vectoredit::WHit>,
    pub lasso: Vec<[f64; 2]>,
    /// The eraser brush under the pointer (x, y, radius in page units).
    pub brush: Option<[f64; 3]>,
    pub face: Vec<Vec<[f64; 2]>>,
    /// Shape builder: the regions under the pointer or collected by the drag (indexes into
    /// [`crate::shapebuilder::current`]) and whether releasing removes them.
    pub builder_regions: Vec<usize>,
    pub builder_remove: bool,
    /// Measuring (Alt): the box of the layer or frame measured to (x0, y0, x1, y1), outlined.
    pub measure_box: Option<[f64; 4]>,
    /// Scale tool drag: the factor so far and the point that stays put.
    pub scale: Option<ScaleHint>,
    /// The selected ellipse's arc handle under the pointer, and the readout while one is dragged
    /// (see [`crate::arc_handles`]).
    pub arc_hover: Option<ArcHandle>,
    pub arc: Option<ArcHint>,
    /// The selected shape's corner radius or star ratio handle under the pointer, and the readout
    /// while one is dragged (see [`crate::shape_handles`]).
    pub shape_hover: Option<ShapeHandle>,
    pub shape: Option<ShapeHint>,
}

/// A point a press would add to the edited vector's outline: page position, whether it is a
/// segment's midpoint (the Move tool) rather than the pen's place on its segment, and whether the
/// pointer is on it (a press adds it; otherwise it just shows where one would go).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct VectorAdd {
    pub at: [f64; 2],
    pub midpoint: bool,
    pub on: bool,
}

/// The Scale tool's live readout while dragging a handle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScaleHint {
    pub factor: f64,
    pub pivot: [f64; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct ResizeStart {
    /// Node id → (world transform, w, h) at the start.
    nodes: Vec<(NodeId, Affine, f64, f64)>,
    /// The handle box: page transform of its top-left corner (axes along the box) and its size.
    frame: (Affine, f64, f64),
    /// One layer (resized in place) rather than several (mapped from the old box to the new).
    single: bool,
    handle: Handle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveDraw {
    /// Only the place changes: grab the layers once, then put that bitmap down moved.
    Translate,
    /// The layers themselves change (a resize, a turn): grab them again every frame, or the
    /// bitmap would be stretched out of shape.
    Reshape,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum Gesture {
    #[default]
    None,
    Marquee {
        start: Point,
        /// Shift: toggle what it touches in the selection it started from (`base`).
        additive: bool,
        base: Vec<NodeId>,
        within: Option<NodeId>,
    },
    Move {
        start: Point,
        ids: Vec<NodeId>,
        /// What the drag actually moves, which Alt turns into copies of `ids` (new layers every
        /// event): the canvas draws these, so it's these the render must leave out.
        live: Vec<NodeId>,
        targets: Targets,
        duplicate: bool,
        moved: bool,
        /// A Shift-press on this selected layer: it leaves the selection on release, unless the
        /// press became a drag.
        deselect: Option<NodeId>,
        /// A press on one layer of a multi-selection that keeps the selection to drag it all: a
        /// release without a drag selects just this layer.
        keep: Option<NodeId>,
    },
    Resize {
        start: Point,
        r: Box<ResizeStart>,
        scale_tool: bool,
        moved: bool,
    },
    Rotate {
        center: Point,
        start_angle: f64,
        ids: Vec<NodeId>,
        /// The pointer has turned since the press: only then is the angle (and the pixels) changing.
        moved: bool,
    },
    Draw {
        start: Point,
        id: Option<NodeId>,
        parent: NodeId,
        targets: Targets,
    },
    Pan {
        sx: f64,
        sy: f64,
        cx: f64,
        cy: f64,
    },
    Pencil {
        points: Vec<Point>,
        /// The pen pressure at each point, where the device reports it.
        pressure: Vec<Option<f32>>,
    },
    TextSelect {
        id: NodeId,
    },
    /// A new text from the Text tool, still pressed: a drag makes it a box the size of the drag
    /// (at least `line`, the height of its first line, tall).
    TextBox {
        id: NodeId,
        start: Point,
        line: f64,
    },
    PenDrag {
        anchor: Point,
    },
    /// Vector edit mode gestures (handled in `vectoredit`).
    Vector(crate::vectoredit::VGesture),
    /// Dragging an ellipse's arc handle: the arc so far (see [`crate::arc_handles`]).
    Arc {
        id: NodeId,
        handle: ArcHandle,
        arc: crate::arc_handles::Arc,
    },
    /// Dragging a shape's corner radius or star ratio handle (see [`crate::shape_handles`]).
    Shape {
        id: NodeId,
        handle: ShapeHandle,
    },
    /// Dragging the rotation origin handle (see [`crate::rotation_origin`]).
    Origin,
    /// The connector tool: dragging from `from` (pressed at `start`).
    Connect {
        from: figcraft_doc::End,
        start: Point,
    },
    /// Dragging the selected connector `id`'s end (`from`: its start, pressed at `start`) to
    /// attach it elsewhere.
    Reattach {
        id: NodeId,
        from: bool,
        start: Point,
    },
    /// Dragging the selected connector `id`'s path handle for `portion` (pressed at `start`).
    Reshape {
        id: NodeId,
        portion: usize,
        start: Point,
    },
    /// The eraser: the strokes it took so far.
    Erase {
        gone: Vec<NodeId>,
    },
    /// Shape builder: regions collected so far (handled in `shapebuilder`).
    ShapeBuild {
        picked: Vec<usize>,
        remove: bool,
        start: Point,
        last: Point,
    },
}

/// Where Shift puts the Pen's next point: as far from `last` as `p` is, at the nearest 45° step.
fn at_45_degrees(last: Point, p: Point) -> Point {
    let v = p - last;
    let ang = (v.atan2() / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
    last + Vec2::from_angle(ang) * v.hypot()
}

/// The handles a pen path shows while it is drawn ([`Overlay::pen_handles`]): the last anchor's in
/// and out handles and the one before's out handle, those that aren't zero.
fn pen_handles(points: &[(Point, Vec2, Vec2)]) -> Vec<[f64; 4]> {
    let last = points.last().into_iter().flat_map(|(p, i, o)| [(p, i), (p, o)]);
    let before = points.len().checked_sub(2).and_then(|k| points.get(k)).into_iter().map(|(p, _, o)| (p, o));
    before.chain(last).filter(|(_, h)| h.hypot() > 0.0).map(|(p, h)| [p.x, p.y, p.x + h.x, p.y + h.y]).collect()
}

/// The pen tool's in-progress path (page coordinates).
#[derive(Clone, Debug, Default)]
pub struct PenPath {
    pub id: Option<NodeId>,
    pub parent: Option<NodeId>,
    /// Anchors with (in, out) handle offsets.
    pub points: Vec<(Point, Vec2, Vec2)>,
    pub closed: bool,
    /// The path ends connected to this anchor of its own (a vector network).
    pub join: Option<usize>,
    /// Anchors taken back by undo, the last one first to come back on redo.
    pub undone: Vec<(Point, Vec2, Vec2)>,
}

/// How the Pencil dashes a stroke (ported from lizard `crates/engine/src/tools.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawLineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl DrawLineStyle {
    pub fn id(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Dashed => "dashed",
            Self::Dotted => "dotted",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "solid" => Some(Self::Solid),
            "dashed" => Some(Self::Dashed),
            "dotted" => Some(Self::Dotted),
            _ => None,
        }
    }
    pub fn dashes(self, weight: f64) -> Vec<f64> {
        match self {
            Self::Solid => Vec::new(),
            Self::Dashed => vec![weight * 3.0, weight * 2.0],
            Self::Dotted => vec![0.1, weight * 1.5],
        }
    }
}

/// Settings for the next Pen / Pencil stroke (ported from lizard's `DrawStyle`; figcraft has no
/// `ToolId::Brush`, so the brush preset stays in `ToolState::pencil_brush` / `tool.pencilBrush`).
/// `None` means the design default in `PENCIL_STROKE`.
#[derive(Clone, Debug, Default)]
pub struct DrawStyle {
    pub color: Option<figcraft_color::Color>,
    pub weight: Option<f64>,
    pub line_style: DrawLineStyle,
}

impl DrawStyle {
    pub fn color(&self) -> figcraft_color::Color {
        self.color.unwrap_or(PENCIL_STROKE.0)
    }
    pub fn weight(&self, _tool: ToolId) -> f64 {
        self.weight.unwrap_or(PENCIL_STROKE.1)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolState {
    pub tool: ToolId,
    pub(crate) gesture: Gesture,
    pub overlay: Overlay,
    pub pen: Option<PenPath>,
    /// Tool to return to after a temporary tool (space → hand).
    pub temp_from: Option<ToolId>,
    /// Last click (for double-click detection by agents that only send downs).
    pub last_hover: Option<NodeId>,
    /// Shape builder: the cached arrangement of the selection, and Remove mode (a plain click
    /// removes; Alt merges).
    pub shape_builder: Option<std::sync::Arc<crate::shapebuilder::Arrangement>>,
    pub builder_remove: bool,
    /// Dev Mode: the canvas inspects. Hovering shows distances from the selection without Alt, and
    /// presses select, pan and comment but never edit the design (no moving, resizing, drawing,
    /// or editing text or vectors).
    pub dev_mode: bool,
    /// Pen snap targets, kept while hovering (see `vectoredit::point_snap`).
    pub(crate) pen_targets: Option<crate::vectoredit::point_snap::PenTargets>,
    /// Vector edit mode's pen: where the last press that started something new (no path being
    /// drawn) was, and the undo depth before it, so a double-click there takes that edit back.
    pub(crate) pen_press: Option<(Point, usize)>,
    /// The Scale tool's anchor: column and row (0 start, 1 middle, 2 end) of the point of the
    /// selection box that stays put (`tool.scaleAnchor`).
    pub scale_anchor: (u8, u8),
    /// The last drag move handled (a held button repeats it every frame).
    pub(crate) last_move: Option<PointerEvent>,
    /// The name labels of top-level frames and sections as the canvas last drew them (each label's
    /// corners on the page, turned with its frame): the Move and Scale tools treat a press or
    /// hover on one as on its frame.
    pub labels: Vec<(NodeId, [Point; 4])>,
    /// The brush the Pencil draws with (`tool.pencilBrush`; `None`: a plain stroke).
    pub pencil_brush: Option<figcraft_doc::Brush>,
    /// Settings for the next Pen / Pencil stroke (`tool.drawStyle`, lizard `DrawStyle`).
    pub pen_style: DrawStyle,
    pub pencil_style: DrawStyle,
    /// The board tools' colours and choices.
    pub board: crate::board::BoardTool,
    /// The eyedropper samples for a colour picker (`edit.pickColor` with `apply: false`): the
    /// colour goes to `picked`, for the picker to write to the paint it edits, not to the fill.
    pub pick_only: bool,
    /// The colour the eyedropper sampled for a colour picker, until the picker takes it.
    pub picked: Option<figcraft_color::Color>,
    /// The widget layers whose hover styles show while the pointer is over them (a live preview).
    pub(crate) widget_hover: Vec<NodeId>,
    /// The placement crosshair: where Enter puts the next shape or frame once the arrows have
    /// shown it (in the middle of the view) and moved it (`tool.crosshair`), in page coordinates
    /// of the document and page in `place_on` (elsewhere it isn't shown: `Session::crosshair`).
    /// Placing, Escape and a change of tool take it away.
    pub place_at: Option<Point>,
    /// The document (uid) and page the crosshair was shown on.
    pub(crate) place_on: Option<(u64, NodeId)>,
}

impl ToolState {
    /// The per-tool drawing style (`Pen` / `Pencil` only); mirrors lizard `draw_style`.
    pub fn draw_style(&self, tool: ToolId) -> Option<&DrawStyle> {
        match tool {
            ToolId::Pen => Some(&self.pen_style),
            ToolId::Pencil => Some(&self.pencil_style),
            _ => None,
        }
    }
    pub fn draw_style_mut(&mut self, tool: ToolId) -> Option<&mut DrawStyle> {
        match tool {
            ToolId::Pen => Some(&mut self.pen_style),
            ToolId::Pencil => Some(&mut self.pencil_style),
            _ => None,
        }
    }

    pub fn busy(&self) -> bool {
        !matches!(self.gesture, Gesture::None)
    }

    /// The layers a gesture has already taken hold of: the canvas may draw them itself (and ask
    /// the renderer to leave them out, `figcraft_render::Options::skip`) so what it draws keeps up
    /// with the selection box instead of waiting for the next render to land.
    ///
    /// Empty until the drag actually changes something: a plain click changes nothing on the page,
    /// and taking the layers out of the render for it would only cost a render.
    ///
    /// [`LiveDraw::Translate`] needs one grab for the whole gesture (a move only shifts the
    /// bitmap); [`LiveDraw::Reshape`] needs a fresh one every frame (a stretch or a turn redraws
    /// the layers themselves).
    pub fn live_layers(&self) -> Option<(Vec<NodeId>, LiveDraw)> {
        match &self.gesture {
            Gesture::Move { live, moved: true, .. } if !live.is_empty() => Some((live.clone(), LiveDraw::Translate)),
            // A multi-layer resize is mapped from the old box to the new one; both are drawn as
            // one stand-in (the grab walks the page, so every layer keeps its own frame's clip).
            Gesture::Resize { r, moved: true, .. } => {
                let ids: Vec<NodeId> = r.nodes.iter().map(|n| n.0).collect();
                (!ids.is_empty()).then_some((ids, LiveDraw::Reshape))
            }
            Gesture::Rotate { ids, moved: true, .. } if !ids.is_empty() => Some((ids.clone(), LiveDraw::Reshape)),
            _ => None,
        }
    }

    /// Whether the live reshape reflows its parents and siblings: a resize does (a hugging frame
    /// grows with the child, the layout pushes its siblings along), a turn only changes the
    /// layer's own geometry. The canvas then draws the box layout gives the layer — the frame
    /// that hugs it and the siblings it moves — instead of the layer on its own, so the frame
    /// follows in the same frame instead of a render behind.
    pub fn live_reflows(&self) -> bool {
        matches!(self.gesture, Gesture::Resize { moved: true, .. })
    }
}

#[cfg(test)]
mod draw_style_tests {
    use super::*;

    /// The ported lizard defaults / dash patterns and the per-tool style slots.
    #[test]
    fn draw_style_defaults_and_dashes() {
        assert_eq!(DrawLineStyle::default(), DrawLineStyle::Solid);
        assert_eq!(DrawLineStyle::Solid.id(), "solid");
        assert_eq!(DrawLineStyle::from_id("dotted"), Some(DrawLineStyle::Dotted));
        assert_eq!(DrawLineStyle::Dashed.dashes(2.0), vec![6.0, 4.0]);
        assert_eq!(DrawLineStyle::Dotted.dashes(2.0), vec![0.1, 3.0]);
        assert!(DrawLineStyle::Solid.dashes(2.0).is_empty());

        let s = DrawStyle::default();
        assert_eq!(s.color().to_css_hex(), PENCIL_STROKE.0.to_css_hex());
        assert_eq!(s.weight(ToolId::Pencil), PENCIL_STROKE.1);

        let t = ToolState::default();
        assert!(t.draw_style(ToolId::Pen).is_some());
        assert!(t.draw_style(ToolId::Pencil).is_some());
        assert!(t.draw_style(ToolId::Move).is_none());
    }
}
