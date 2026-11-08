//! 阶段0 验证入口。
//!
//! 在**有显示设备的桌面环境**里，从本 crate 目录运行即可看到 GPUI 画布窗口：
//!
//! ```text
//! cd crates/ui-gpui
//! cargo run
//! ```
//!
//! 窗口里显示 engine 渲染出来的文档页面，可鼠标拖拽平移、滚轮缩放——实证
//! "GPUI 收输入 → engine 渲染 → GPUI 显示" 整条回路。
//!
//! 注意：**无头环境（CI / 无显示服务器）会因无法创建窗口而启动失败**，这是预期的，
//! 不是代码问题。本 crate 被隔离在 workspace `exclude` 之外，单独 `cargo run` 即可，
//! 不影响主仓库的 egui 构建。

fn main() {
    figcraft_ui_gpui::run();
}
