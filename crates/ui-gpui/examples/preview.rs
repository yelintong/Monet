//! 无 GPU / 无显示环境下预览画布：把窗口实际会显示的内容合成一张 PNG。
//!
//! 运行：
//! ```text
//! cd crates/ui-gpui
//! cargo run --example preview
//! ```
//! 会在当前目录写出 `canvas_preview.png`（含顶部/左侧标尺、选中框、8 手柄、框选框、对齐参考线），
//! 用任意图片查看器打开即可看到画布长什么样——等价于在真机跑窗口看到的内容。

fn main() {
    match figcraft_ui_gpui::export_preview("canvas_preview.png") {
        Ok(()) => println!("已写出 canvas_preview.png（用图片查看器打开即可看到画布）"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
