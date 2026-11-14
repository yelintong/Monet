//! lizard 图层树字形：`crates/ui-egui/assets/layer-*.svg` 的原生 SVG，1:1 内嵌。
//! 上游 `icons_ui3::paint_layer` 用 16×16 viewBox 画这些字；GPUI 的 SvgRenderer 只取
//! alpha，颜色由 `.text_color()` 给，所以 SVG 里的白色描边/填充正好能被统一着色。

use gpui::{Hsla, IntoElement, SharedString, prelude::*, px, svg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerIcon {
    Frame,
    LayoutH,
    LayoutV,
    LayoutGrid,
    Text,
    Absolute,
    Mask,
    Ellipse,
    EllipseMask,
    Component,
    Instance,
    Vector,
    Image,
    Star,
    Polygon,
    Arrow,
    Line,
    Rectangle,
    Group,
    CollapseAll,
}

impl LayerIcon {
    pub fn path(self) -> SharedString {
        SharedString::from(format!("layer/{}.svg", ASSETS[self as usize].0))
    }
}

/// (file stem, bytes). Order MUST match the enum above.
pub static ASSETS: &[(&str, &[u8])] = &[
    ("frame", include_bytes!("../../assets/layer/frame.svg")),
    ("layout-horizontal", include_bytes!("../../assets/layer/layout-horizontal.svg")),
    ("layout-vertical", include_bytes!("../../assets/layer/layout-vertical.svg")),
    ("layout-grid", include_bytes!("../../assets/layer/layout-grid.svg")),
    ("text", include_bytes!("../../assets/layer/text.svg")),
    ("absolute", include_bytes!("../../assets/layer/absolute.svg")),
    ("mask", include_bytes!("../../assets/layer/mask.svg")),
    ("ellipse", include_bytes!("../../assets/layer/ellipse.svg")),
    ("ellipse-mask", include_bytes!("../../assets/layer/ellipse-mask.svg")),
    ("component", include_bytes!("../../assets/layer/component.svg")),
    ("instance", include_bytes!("../../assets/layer/instance.svg")),
    ("vector", include_bytes!("../../assets/layer/vector.svg")),
    ("image", include_bytes!("../../assets/layer/image.svg")),
    ("star", include_bytes!("../../assets/layer/star.svg")),
    ("polygon", include_bytes!("../../assets/layer/polygon.svg")),
    ("arrow", include_bytes!("../../assets/layer/arrow.svg")),
    ("line", include_bytes!("../../assets/layer/line.svg")),
    ("rectangle", include_bytes!("../../assets/layer/rectangle.svg")),
    ("group", include_bytes!("../../assets/layer/group.svg")),
    ("collapse", include_bytes!("../../assets/layer/collapse.svg")),
];

/// Draw one layer glyph in a single ink colour, at its native 16 pt size.
pub fn paint(icon: LayerIcon, color: gpui::Rgba, size: f32) -> impl IntoElement {
    svg().path(icon.path()).w(px(size)).h(px(size)).flex_none().text_color(Hsla::from(color))
}

/// AssetSource hook: map `layer/<stem>.svg` onto the embedded bytes.
pub fn load(path: &str) -> Option<&'static [u8]> {
    let stem = path.strip_prefix("layer/")?.strip_suffix(".svg")?;
    ASSETS.iter().find(|(name, _)| *name == stem).map(|(_, bytes)| *bytes)
}

pub fn paths() -> Vec<SharedString> {
    ASSETS.iter().map(|(name, _)| SharedString::from(format!("layer/{name}.svg"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_glyph_has_an_asset() {
        assert_eq!(ASSETS.len(), 20);
        for (stem, bytes) in ASSETS {
            assert!(!bytes.is_empty(), "{stem} is empty");
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.contains("<svg"), "{stem} is not an SVG");
            // GPUI renders SVGs through usvg; a parse failure would silently draw nothing.
            assert!(usvg::Tree::from_data(bytes, &usvg::Options::default()).is_ok(), "{stem} does not parse");
        }
    }
}
