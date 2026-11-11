//! 排版对话框专用的矢量字形（照 lizard `ui-egui/assets/typ-*.svg` 1:1 移植）。
//!
//! 为什么不放进冻结的 [`super::icon::Icon`] 枚举：那张表是 79/91 个「工具 / 面板」图标，
//! 判别值、`asset_name`、`ALL`、单测断言全部锁死；而这一组 22 个只服务 Type settings 的
//! 分段控件（U 带下划线、S 带删除线、AG/ag/Ag/Ao、列表项目符号 / 编号、垂直裁剪…），
//! 语义上属于「对话框字形」而非工具图标。项目既有约定（2026-11-11）：冻结文件只加
//! `pub mod` / `pub use`，不改内容。于是走 `canvas/svg_text.rs` 里 `AppAssets` 的 `typ:`
//! 前缀，把 SVG 字节直接内联，用与 `icon::paint` 完全相同的 `svg().path().text_color()`
//! 管线渲染 —— 着色由 `.text_color()` 注入，所以这里只收录黑色矢量。

use gpui::{Hsla, IntoElement, SharedString, prelude::*, svg};

/// 排版对话框字形。每个对应一个 `typ*.svg` 资源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypIcon {
    // ── 对齐（4 格，Figma DOM: LEFT / CENTER / RIGHT / JUSTIFIED）──
    AlignLeft,
    AlignCenter,
    AlignRight,
    Justify,
    // ── 文本修饰（3 格 + drill-in）──
    None,
    Underline,
    Strike,
    // ── 大小写（5 格）──
    Upper,
    Lower,
    Title,
    SmallCaps,
    // ── 垂直裁剪（2 格）──
    TrimStandard,
    TrimCap,
    // ── 列表样式（3 格）──
    ListBullets,
    ListNumbered,
    // ── 截断文本（2 格）──
    Truncate,
    // ── 右栏字体排印行（预留，本轮不接）──
    LineHeight,
    LetterSpacing,
    Settings,
    // ── 文本垂直对齐（6 格，预留）──
    VTop,
    VMiddle,
    VBottom,
}

/// 资源表里的一项：`TypIcon` → 资源名（`typ:<name>.svg`）+ 内联 SVG 字节。
struct Entry {
    icon: TypIcon,
    name: &'static str,
    svg: &'static str,
}

/// 唯一资源表。SVG 来自 lizard，用 `include_str!` 编译期内联，运行时零 I/O。
static ENTRIES: &[Entry] = &[
    Entry { icon: TypIcon::AlignLeft, name: "typ-align-left", svg: include_str!("../../assets/typ/typ-align-left.svg") },
    Entry { icon: TypIcon::AlignCenter, name: "typ-align-center", svg: include_str!("../../assets/typ/typ-align-center.svg") },
    Entry { icon: TypIcon::AlignRight, name: "typ-align-right", svg: include_str!("../../assets/typ/typ-align-right.svg") },
    Entry { icon: TypIcon::Justify, name: "typ-dialog-justify", svg: include_str!("../../assets/typ/typ-dialog-justify.svg") },
    Entry { icon: TypIcon::None, name: "typ-dialog-none", svg: include_str!("../../assets/typ/typ-dialog-none.svg") },
    Entry { icon: TypIcon::Underline, name: "typ-dialog-underline", svg: include_str!("../../assets/typ/typ-dialog-underline.svg") },
    Entry { icon: TypIcon::Strike, name: "typ-dialog-strike", svg: include_str!("../../assets/typ/typ-dialog-strike.svg") },
    Entry { icon: TypIcon::Upper, name: "typ-dialog-upper", svg: include_str!("../../assets/typ/typ-dialog-upper.svg") },
    Entry { icon: TypIcon::Lower, name: "typ-dialog-lower", svg: include_str!("../../assets/typ/typ-dialog-lower.svg") },
    Entry { icon: TypIcon::Title, name: "typ-dialog-title", svg: include_str!("../../assets/typ/typ-dialog-title.svg") },
    Entry { icon: TypIcon::SmallCaps, name: "typ-dialog-small-caps", svg: include_str!("../../assets/typ/typ-dialog-small-caps.svg") },
    Entry { icon: TypIcon::TrimStandard, name: "typ-dialog-trim-standard", svg: include_str!("../../assets/typ/typ-dialog-trim-standard.svg") },
    Entry { icon: TypIcon::TrimCap, name: "typ-dialog-trim-cap", svg: include_str!("../../assets/typ/typ-dialog-trim-cap.svg") },
    Entry { icon: TypIcon::ListBullets, name: "typ-dialog-list-bullets", svg: include_str!("../../assets/typ/typ-dialog-list-bullets.svg") },
    Entry { icon: TypIcon::ListNumbered, name: "typ-dialog-list-numbered", svg: include_str!("../../assets/typ/typ-dialog-list-numbered.svg") },
    Entry { icon: TypIcon::Truncate, name: "typ-dialog-truncate", svg: include_str!("../../assets/typ/typ-dialog-truncate.svg") },
    Entry { icon: TypIcon::LineHeight, name: "typ-line-height", svg: include_str!("../../assets/typ/typ-line-height.svg") },
    Entry { icon: TypIcon::LetterSpacing, name: "typ-letter-spacing", svg: include_str!("../../assets/typ/typ-letter-spacing.svg") },
    Entry { icon: TypIcon::Settings, name: "typ-settings", svg: include_str!("../../assets/typ/typ-settings.svg") },
    Entry { icon: TypIcon::VTop, name: "typ-align-top", svg: include_str!("../../assets/typ/typ-align-top.svg") },
    Entry { icon: TypIcon::VMiddle, name: "typ-align-middle", svg: include_str!("../../assets/typ/typ-align-middle.svg") },
    Entry { icon: TypIcon::VBottom, name: "typ-align-bottom", svg: include_str!("../../assets/typ/typ-align-bottom.svg") },
];

/// `AssetSource` 前缀：与 `canvas/svg_text.rs` 的 `PREFIX = "text:"` 平级。
pub const ASSET_PREFIX: &str = "typ:";

impl TypIcon {
    /// 生成 `svg().path()` 用的资源路径。
    pub fn asset_path(self) -> SharedString {
        let e = self.entry();
        SharedString::from(format!("{ASSET_PREFIX}{}.svg", e.name))
    }

    /// 供 `AppAssets::load` 查表：`typ:<name>.svg` → SVG 字节。
    pub fn load(name: &str) -> Option<&'static str> {
        ENTRIES.iter().find(|e| e.name == name).map(|e| e.svg)
    }

    /// 供 `AppAssets::list` 枚举全部路径。
    pub fn list() -> impl Iterator<Item = SharedString> {
        ENTRIES.iter().map(|e| SharedString::from(format!("{ASSET_PREFIX}{}.svg", e.name)))
    }

    fn entry(self) -> &'static Entry {
        ENTRIES.iter().find(|e| e.icon == self).expect("TypIcon 与资源表不同步")
    }
}

/// 与 `icon::paint` 同式：把字形画成 `size` px 的元素，颜色由 `text_color` 注入。
pub fn paint(icon: TypIcon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    let rgba: gpui::Rgba = color.into();
    svg()
        .path(icon.asset_path())
        .w(gpui::px(size))
        .h(gpui::px(size))
        .flex_none()
        .text_color(Hsla::from(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_typ_icon_has_inline_svg_and_unique_path() {
        use std::collections::HashSet;
        let mut paths = HashSet::new();
        for e in ENTRIES {
            assert!(e.svg.contains("<svg"), "{} 不是 SVG", e.name);
            assert!(paths.insert(e.name), "重复资源名 {}", e.name);
        }
        assert_eq!(ENTRIES.len(), 22, "排版字形表变了：请同步本断言");
    }

    #[test]
    fn load_roundtrips_every_path() {
        for e in ENTRIES {
            let path = e.icon.asset_path();
            let name = path.strip_prefix(ASSET_PREFIX).unwrap().strip_suffix(".svg").unwrap();
            assert_eq!(TypIcon::load(name), Some(e.svg));
        }
        assert!(TypIcon::load("no-such-glyph").is_none());
    }
}
