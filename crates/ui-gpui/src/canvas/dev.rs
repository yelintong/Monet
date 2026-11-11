//! Dev Mode 在**画布**上的那一半（egui `ui-egui/src/panels/dev_mode.rs` 的 canvas 段落）：
//! - 已存测量线（`saved_lines`，与 hover 的智能参考线一起画在 `measure_overlay` 里）；
//! - 每条注解的 callout（分类 pill / 便签 / 钉住的属性），紧跟图层右上角、互相不重叠。
//!
//! 聚焦视图的顶栏是 retained UI 元素，在 `lib.rs::dev_focus_bar` 里，不在这个画笔里。
//!
//! 与 egui 的差异：`Painter` 只有「单行标签」，没有带自动换行的 `LayoutJob`，
//! 所以便签在这里按估算字宽手动折行（最多 6 行）；卡片的投影用一层半透明矩形近似。

use figcraft_engine::{annotations, dev_measurements};
use figcraft_engine::snap::Guide;

use super::geom::*;
use super::painter::{Painter, text_size};
use crate::app::CanvasApp;
use crate::theme::{Tokens, accent, doc_c32};

/// 卡片宽度与到图层的间距（egui `CALLOUT_W` / `CALLOUT_GAP`）。
const CALLOUT_W: f32 = 220.0;
const CALLOUT_GAP: f32 = 24.0;
const CATEGORY_H: f32 = 18.0;

/// Dev Mode 是否激活（Inspect 标签且不在演示模式；GPUI 没有演示模式，只判标签）。
pub(crate) fn active(app: &CanvasApp) -> bool {
    app.dev_inspect
}

/// 当前页留在画布上的测量线，按 egui `saved_lines` 的过滤：
/// 起点（和终点）都可见，专注视图里只画它那个 frame 的后代。
pub(crate) fn saved_lines(app: &CanvasApp) -> Vec<Guide> {
    if !active(app) {
        return Vec::new();
    }
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let focus = ds.focused();
    dev_measurements::on_page(&ds.doc, ds.page)
        .into_iter()
        .filter(|m| {
            let shown = |id: figcraft_doc::NodeId| ds.doc.node(id).is_some_and(|n| n.visible);
            shown(m.from) && m.measurement.to.is_none_or(shown) && focus.is_none_or(|f| ds.doc.is_ancestor(f, m.from))
        })
        .flat_map(|m| m.lines)
        .collect()
}

/// 一条注解的卡片内容（预先算好高度，才好互相避让）。
struct Callout {
    category: Option<String>,
    note: Vec<String>,
    note_h: f32,
    height: f32,
}

impl Callout {
    fn new(painter: &Painter, label: &str, category: Option<String>, pins: usize) -> Self {
        let note = wrap_note(painter, label, CALLOUT_W - 22.0, regular(11.0), 6);
        let line_h = text_size("", regular(11.0)).y;
        let note_h = note.len() as f32 * line_h;
        let gap = if !note.is_empty() && pins > 0 { 6.0 } else { 0.0 };
        let cat_h = if category.is_some() { CATEGORY_H } else { 0.0 };
        let height = cat_h + note_h + gap + pins as f32 * 18.0 + 16.0;
        Self { category, note, note_h, height }
    }
}

/// 便签按估算字宽折行（egui `LayoutJob::simple` + `wrap.max_rows = 6` 的等价）。
fn wrap_note(painter: &Painter, text: &str, width: f32, font: FontId, max_rows: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if painter.layout_no_wrap(candidate.clone(), font, Color32::WHITE).size.x <= width || cur.is_empty() {
            cur = candidate;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
        if lines.len() == max_rows {
            break;
        }
    }
    if lines.len() < max_rows && !cur.is_empty() {
        lines.push(cur);
    }
    // 截断了就给最后一行加省略号（egui 的 `max_rows` 也是省略）。
    if text.split_whitespace().count() > 0 && lines.len() == max_rows {
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines
}

/// 画本页所有可见注解的卡片（egui `dev_mode::draw`）。
pub(crate) fn draw_annotations(app: &CanvasApp, painter: &mut Painter, t: &Tokens) {
    if !active(app) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let doc = &ds.doc;
    let focus = ds.focused();
    let clip = painter.clip;
    let mut placed: Vec<Rect> = Vec::new();

    for id in annotations::on_page(doc, ds.page) {
        if focus.is_some_and(|f| !doc.is_ancestor(f, id)) {
            continue;
        }
        let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
        let Some(b) = doc.world_bounds(id) else { continue };
        let r = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
        let x = r.max.x + CALLOUT_GAP;
        if x > clip.max.x || x + CALLOUT_W < clip.min.x || r.min.y > clip.max.y {
            continue;
        }
        let selected = ds.selection.contains(&id);
        let anchor = pos2(r.max.x, r.min.y);
        let mut y = r.min.y;
        let mut first = true;
        for a in n.annotations.iter().filter(|a| filter_shows(doc, a, &app.dev_cat_filter)) {
            if y > clip.max.y {
                break;
            }
            let pinned = annotations::pinned(doc, id, a);
            let category = annotations::category_of(doc, a);
            let color = category.map_or_else(|| accent(t), |c| doc_c32(c.color));
            let card = Callout::new(painter, &a.label, category.map(|c| c.name.clone()), pinned.len());
            let mut rect = Rect::from_min_size(pos2(x, y), vec2(CALLOUT_W, card.height));
            while let Some(below) = placed.iter().filter(|p| p.intersects(rect.expand(3.0))).map(|p| p.max.y).reduce(f32::max) {
                rect = rect.translate(vec2(0.0, below + 6.0 - rect.min.y));
            }
            if first {
                painter.line_segment([anchor, pos2(rect.min.x, rect.min.y + 14.0)], Stroke::new(1.0, color));
                painter.circle_filled(anchor, 3.5, color);
                first = false;
            }
            paint_callout(painter, t, &card, rect, color, &pinned, selected);
            placed.push(rect);
            y = rect.max.y + 6.0;
        }
    }
}

/// egui `CategoryFilter::shows`：`None` 全部 / `Some("")` 无分类 / `Some(id)` 指定分类。
fn filter_shows(doc: &figcraft_doc::Document, a: &figcraft_doc::Annotation, filter: &Option<String>) -> bool {
    let cat = annotations::category_of(doc, a);
    match filter {
        None => true,
        Some(id) if id.is_empty() => cat.is_none(),
        Some(id) => cat.is_some_and(|c| c.id == *id),
    }
}

/// 一张卡片：投影 / 底 / 左边色条 / 分类 pill / 便签 / 钉住的属性。
fn paint_callout(
    painter: &mut Painter,
    t: &Tokens,
    card: &Callout,
    rect: Rect,
    color: Color32,
    pinned: &[(&'static annotations::Pinnable, Option<figcraft_engine::devinspect::Prop>)],
    selected: bool,
) {
    // egui 用 `epaint::Shadow`；GPUI 的画笔没有模糊，用一层半透明偏移矩形近似。
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)).expand(3.0), CornerRadius::same(6), Color32::from_black_alpha(28));
    painter.rect_filled(rect, CornerRadius::same(6), t.panel);
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, if selected { accent(t) } else { t.border_strong }), StrokeKind::Inside);
    painter.rect_filled(
        Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, rect.height() - 12.0)),
        CornerRadius::same(2),
        color,
    );
    let x = rect.min.x + 12.0;
    let mut y = rect.min.y + 8.0;
    if let Some(name) = &card.category {
        let g = painter.layout_no_wrap(name.clone(), semibold(9.0), Color32::WHITE);
        let w = g.size.x.min(CALLOUT_W - 30.0);
        let pill = Rect::from_min_size(pos2(x, y - 1.0), vec2(w + 10.0, 14.0));
        painter.rect_filled(pill, CornerRadius::same(4), color);
        painter.galley(pos2(pill.min.x + 5.0, pill.min.y + (14.0 - g.size.y) / 2.0), g, Color32::WHITE);
        y += CATEGORY_H;
    }
    if !card.note.is_empty() {
        let line_h = text_size("", regular(11.0)).y;
        for line in &card.note {
            painter.text(pos2(x, y), Align2::LEFT_TOP, line.clone(), regular(11.0), t.text);
            y += line_h;
        }
        y += if pinned.is_empty() { 0.0 } else { 6.0 };
        let _ = card.note_h;
    }
    for (p, v) in pinned {
        let mid = y + 9.0;
        painter.text(pos2(x, mid), Align2::LEFT_CENTER, p.label, regular(10.0), t.text_secondary);
        let mut vx = x + 84.0;
        if let Some(c) = v.as_ref().and_then(|v| v.color.as_deref()).and_then(figcraft_color::Color::from_hex) {
            painter.rect_filled(Rect::from_center_size(pos2(vx + 5.0, mid), vec2(10.0, 10.0)), CornerRadius::same(2), doc_c32(c));
            vx += 15.0;
        }
        // 样式 / 变量名比裸值更好读（egui 同款：token 优先）。
        let value = v.as_ref().map(|v| v.token.clone().unwrap_or_else(|| v.value.clone())).unwrap_or_else(|| "—".into());
        painter.text(pos2(vx, mid), Align2::LEFT_CENTER, value, regular(10.0), t.text);
        y += 18.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{CanvasApp, sample_doc};
    use crate::theme::Tokens;
    use figcraft_doc::{Document, NodeId};
    use serde_json::json;

    fn setup() -> CanvasApp {
        let mut app = CanvasApp::new(Document::new("dev-mode-test"));
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        app.canvas.rect = rect;
        {
            let Ok(ds) = app.session.ds_mut() else { panic!("no doc state") };
            ds.view.vw = f64::from(rect.width());
            ds.view.vh = f64::from(rect.height());
            ds.zoom_to_fit();
        }
        app
    }

    fn rect_at(app: &mut CanvasApp, x: f64) -> NodeId {
        let v = app.session.execute("shape.rectangle", &json!({ "x": x, "y": 100.0, "width": 60.0, "height": 60.0 })).expect("rect");
        serde_json::from_value(v["id"].clone()).expect("id")
    }

    #[test]
    fn saved_lines_are_drawn_only_in_the_inspect_tab() {
        let mut app = setup();
        let a = rect_at(&mut app, 100.0);
        let b = rect_at(&mut app, 300.0);
        app.session.execute("devMode.addMeasurement", &json!({ "from": a, "to": b })).expect("add measurement");
        app.dev_inspect = false;
        assert!(saved_lines(&app).is_empty(), "Design 标签不画已存测量");
        app.dev_inspect = true;
        let lines = saved_lines(&app);
        assert!(!lines.is_empty(), "Inspect 标签把已存测量画出来");
        assert!(lines.iter().any(|g| g.label.as_deref().is_some_and(|l| !l.is_empty())), "带距离标签");
        // 终点不可见时整条都不画（egui `saved_lines` 的同一道过滤）。
        app.session.execute("node.set", &json!({ "ids": [b], "props": { "visible": false } })).expect("hide");
        assert!(saved_lines(&app).is_empty(), "终点隐藏后不再画");
    }

    #[test]
    fn callouts_appear_and_follow_the_category_filter() {
        let mut app = setup();
        let a = rect_at(&mut app, 140.0);
        app.session.execute("annotation.addCategory", &json!({ "name": "Dev" })).expect("category 1");
        app.session.execute("annotation.addCategory", &json!({ "name": "A11y" })).expect("category 2");
        let cats: Vec<String> = {
            let ds = app.session.ds().expect("ds");
            annotations::categories(&ds.doc).iter().map(|c| c.id.clone()).collect()
        };
        app.session
            .execute("annotation.add", &json!({ "id": a, "label": "Check the padding on narrow screens", "category": cats[0] }))
            .expect("annotation");
        app.dev_inspect = true;
        let t = Tokens::dark();

        let mut all = Painter::new(app.canvas.rect);
        draw_annotations(&app, &mut all, &t);
        assert!(!all.labels.is_empty(), "注解卡片画出了文字");
        assert!(!all.ops.is_empty(), "注解卡片画出了底/描边");

        app.dev_cat_filter = Some(cats[1].clone());
        let mut other = Painter::new(app.canvas.rect);
        draw_annotations(&app, &mut other, &t);
        assert!(other.labels.is_empty(), "只看另一分类时不画这张卡片");

        app.dev_cat_filter = Some(String::new());
        let mut none = Painter::new(app.canvas.rect);
        draw_annotations(&app, &mut none, &t);
        assert!(none.labels.is_empty(), "只看无分类时不画这张卡片");

        app.dev_cat_filter = None;
        let mut shown = Painter::new(app.canvas.rect);
        draw_annotations(&app, &mut shown, &t);
        assert_eq!(shown.labels.len(), all.labels.len(), "全部分类时恢复");
    }

    #[test]
    fn inactive_annotations_draw_nothing() {
        let mut app = CanvasApp::new(sample_doc());
        app.dev_inspect = false;
        let mut p = Painter::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0)));
        draw_annotations(&app, &mut p, &Tokens::dark());
        assert!(p.ops.is_empty() && p.labels.is_empty());
    }
}
