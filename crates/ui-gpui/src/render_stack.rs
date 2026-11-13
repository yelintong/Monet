//! 在渲染栈上跑一次递归光栅化。
//!
//! 渲染按层递归，深度由文档结构决定（组件树、嵌套组件的组件……），调用方无法预知。而 UI 线程的
//! 栈在 Windows 上只有约 1 MB——默认线程 2 MB，渲染线程 8 MB（[`figcraft_render::RENDER_STACK`]）。
//! 直接在 UI 线程上渲染，浅文档没事、深文档就 `thread '<unknown>' has overflowed its stack`
//! （表现为偶发）。
//!
//! 缩略图、画板模板预览、导出预览、首页帧预览都属这一类：它们都在 UI 线程上调用
//! `figcraft_render::render_node` / `render_area`。统一走这里，就不必每处自己 spawn 一次、
//! 也不必让调用方去猜文档有多深。

use std::sync::Arc;

/// 在 8 MB 的渲染栈上运行 `f`，借出所需数据而不克隆文档。
///
/// 返回 `None` 仅当临时线程创建失败或它 panic——两种情况下调用方都应显示"没有预览"，
/// 而不是在 UI 线程上重试（那正是要避免的崩溃路径）。
pub(crate) fn on_render_stack<R, F>(f: F) -> Option<R>
where
    R: Send,
    F: FnOnce() -> R + Send,
{
    std::thread::scope(|s| {
        std::thread::Builder::new().name("figcraft-render-once".into()).stack_size(figcraft_render::RENDER_STACK).spawn_scoped(s, f).ok()?.join().ok()
    })
}

/// 已经包在 `Arc` 里的文档也走同一条路；给需要 `Arc<Document>` 的渲染入口用。
#[allow(dead_code)]
pub(crate) fn on_render_stack_with_doc<R, F>(doc: Arc<figcraft_doc::Document>, f: F) -> Option<R>
where
    R: Send,
    F: FnOnce(&figcraft_doc::Document) -> R + Send,
{
    on_render_stack(move || f(doc.as_ref()))
}
