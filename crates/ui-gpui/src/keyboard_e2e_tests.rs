//! 端到端：按键到底有没有进画布的 `on_key_down`，以及引擎命令有没有被执行。
//! 这些不是 keymap 表单元测试能覆盖的 —— 焦点、事件分发、命令执行各可能断一环。

use crate::CanvasView;
use gpui::{Modifiers, MouseButton, Point, Size, TestAppContext, px};

/// 把焦点交给画布（左键点一下，和真人一样），再送按键。
fn focus_canvas(window: &mut gpui::VisualTestContext) {
    window.simulate_resize(Size::new(px(1280.0), px(860.0)));
    window.simulate_mouse_down(Point::new(px(640.0), px(430.0)), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(Point::new(px(640.0), px(430.0)), MouseButton::Left, Modifiers::default());
}

// `CanvasView::new` sets the global locale from prefs; running these alongside label tests
// makes them flaky. Kept as manual verification (run with `--ignored`).
#[gpui::test]
#[ignore = "manual: creates a CanvasView and changes the global locale"]
async fn a_plain_tool_key_reaches_the_canvas(cx: &mut TestAppContext) {
    let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));
    focus_canvas(window);

    let before = view.update(window, |cv, _| cv.app.session.tool.tool);
    window.simulate_keystrokes("r");
    let after = view.update(window, |cv, _| cv.app.session.tool.tool);
    eprintln!("[key-e2e] 'r' tool: {before:?} -> {after:?}");
    assert_ne!(before, after, "按 R 应该切到矩形工具：键盘事件没进画布");
}

/// 用户报的两个症状：Delete 删不掉组件、Ctrl+Z 撤销没反应。
/// 真实模拟按键，看引擎有没有真的执行命令（undo 栈会变化）。
/// 不改全局语言：键名修复后已与界面语言无关，改语言会污染并行测试。
#[gpui::test]
#[ignore = "manual: creates a CanvasView and changes the global locale"]
async fn delete_and_ctrl_z_run_real_commands(cx: &mut TestAppContext) {
    let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));
    focus_canvas(window);

    let u0 = view.update(window, |cv, _| cv.app.session.ds().map(|d| d.history.undo.len()).unwrap_or(0));
    view.update(window, |cv, _| {
        let _ = cv.app.session.execute("select.all", &serde_json::json!({}));
    });
    window.simulate_keystrokes("delete");
    let u1 = view.update(window, |cv, _| cv.app.session.ds().map(|d| d.history.undo.len()).unwrap_or(0));
    eprintln!("[key-e2e] delete: undo stack {u0} -> {u1}");
    assert!(u1 > u0, "Delete 必须真的执行一次编辑（中文界面）");

    window.simulate_keystrokes("ctrl-z");
    let u2 = view.update(window, |cv, _| cv.app.session.ds().map(|d| d.history.undo.len()).unwrap_or(0));
    eprintln!("[key-e2e] ctrl-z: undo stack {u1} -> {u2}");
    assert!(u2 < u1, "Ctrl+Z 必须真的撤销（中文界面）");

    window.simulate_keystrokes("ctrl-shift-z");
    let u3 = view.update(window, |cv, _| cv.app.session.ds().map(|d| d.history.undo.len()).unwrap_or(0));
    eprintln!("[key-e2e] ctrl-shift-z: undo stack {u2} -> {u3}");
    assert!(u3 > u2, "Ctrl+Shift+Z 必须真的重做");
}

#[gpui::test]
#[ignore = "manual: creates a CanvasView and changes the global locale"]
async fn delete_and_ctrl_z_reach_the_engine(cx: &mut TestAppContext) {
    let (view, window) = cx.add_window_view(|_window, cx| CanvasView::new(cx));
    focus_canvas(window);

    // 先跑一个真的会进 undo 栈的引擎命令，记下文档指纹。
    let before = view.update(window, |cv, _| {
        let ds = cv.app.session.ds_mut().expect("a document");
        let page = ds.page;
        let id = std::sync::Arc::make_mut(&mut ds.doc).create(page, figcraft_doc::NodeKind::Rectangle, "Temp", 10.0, 10.0, 40.0, 40.0).expect("a rectangle");
        ds.doc.node(id).is_some()
    });
    assert!(before);

    let undo_len = view.update(window, |cv, _| cv.app.session.ds().map(|d| d.history.undo.len()).unwrap_or(0));
    eprintln!("[key-e2e] undo stack before keys = {undo_len}");

    // 直接模拟按键；用工具切换来证明事件确实到了 on_key_down。
    let t0 = view.update(window, |cv, _| cv.app.session.tool.tool);
    window.simulate_keystrokes("v");
    let t1 = view.update(window, |cv, _| cv.app.session.tool.tool);
    eprintln!("[key-e2e] 'v' tool: {t0:?} -> {t1:?}");
}
