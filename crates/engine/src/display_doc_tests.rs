//! `Session::display_doc`：动画模式让命中/悬停跟随「画面上显示的那份」文档。
//!
//! 背景（用户 2026-11-12）：动画模式渲染的是播放头求值后的文档，若命中仍读未求值的
//! `ds.doc`，播放头停在中间时就会「看得见却点不中」。`display_doc` 只覆盖命中几何，
//! 写操作仍走 `ds.doc`，所以不会把编辑写进求值副本。

use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Point;

use crate::Session;

fn session_with_rect() -> (Session, NodeId) {
    let mut s = Session::new();
    let page = s.doc().expect("doc").first_page().expect("page");
    let id = s.doc_mut().expect("doc").create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).expect("rect");
    (s, id)
}

/// 显示文档更大 → 命中按它算；清掉后回到静态文档；`ds.doc` 全程没被改。
#[test]
fn hit_follows_the_display_document() {
    let (mut s, id) = session_with_rect();
    let page = s.doc().expect("doc").first_page().expect("page");
    let static_doc: Document = s.doc().expect("doc").clone();

    // 静态层只有 100×100：点 (500,500) 落空。
    assert_eq!(s.hit(&static_doc, page, Point::new(500.0, 500.0), 1.0), None, "静态文档里没有东西");

    // 显示文档把同一层放大到 1000×1000（模拟动画求值后的几何）。
    let mut shown: Document = static_doc.clone();
    if let Some(n) = shown.node_mut(id) {
        n.width = 1000.0;
        n.height = 1000.0;
    }
    s.display_doc = Some(Arc::new(shown));
    assert_eq!(s.hit(&static_doc, page, Point::new(500.0, 500.0), 1.0), Some(id), "命中应跟随显示文档");

    // 清掉后回到静态文档：同一位置又落空。
    s.display_doc = None;
    assert_eq!(s.hit(&static_doc, page, Point::new(500.0, 500.0), 1.0), None, "清掉后应回到静态文档");

    // 真实文档从未被显示文档污染。
    assert_eq!(s.doc().expect("doc").node(id).map(|n| n.width), Some(100.0), "ds.doc 不该被改");
}

/// `display_doc` 不影响写操作：命中返回的 id 仍指向真实文档里的同一层，尺寸不变。
#[test]
fn the_display_document_never_leaks_into_the_real_one() {
    let (mut s, id) = session_with_rect();
    let page = s.doc().expect("doc").first_page().expect("page");
    let static_doc: Document = s.doc().expect("doc").clone();
    let mut shown: Document = static_doc.clone();
    if let Some(n) = shown.node_mut(id) {
        n.width = 1000.0;
        n.height = 1000.0;
    }
    s.display_doc = Some(Arc::new(shown));

    assert_eq!(s.hit(&static_doc, page, Point::new(500.0, 500.0), 1.0), Some(id));
    // 写操作（选择）走真实文档，尺寸仍是 100。
    s.set_selection(vec![id]);
    assert_eq!(s.selection(), vec![id]);
    assert_eq!(s.doc().expect("doc").node(id).map(|n| n.width), Some(100.0), "真实文档尺寸不变");
}

/// 没有显示文档时（非动画模式），命中与旧行为完全一致。
#[test]
fn without_a_display_document_hit_is_unchanged() {
    let (s, id) = session_with_rect();
    let page = s.doc().expect("doc").first_page().expect("page");
    let static_doc: Document = s.doc().expect("doc").clone();
    assert_eq!(s.hit(&static_doc, page, Point::new(50.0, 50.0), 1.0), Some(id));
    assert_eq!(s.hit(&static_doc, page, Point::new(500.0, 500.0), 1.0), None);
}
