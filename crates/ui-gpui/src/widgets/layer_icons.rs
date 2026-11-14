//! lizard 图层树字形：`crates/ui-egui/assets/layer-*.svg` 的原生 SVG，1:1 内嵌。
//! 上游 `icons_ui3::paint_layer` 用 16×16 viewBox 画这些字；GPUI 的 SvgRenderer 只取
//! alpha，颜色由 `.text_color()` 给，所以 SVG 里的白色描边/填充正好能被统一着色。

use gpui::{Hsla, IntoElement, SharedString, prelude::*, px, svg};
use std::sync::OnceLock;

/// Layer glyphs and shape-tool marks share the very same stroke constant.
pub use crate::widgets::icon::ICON_STROKE;

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

/// The SVG's own viewBox width: the vector occupies the space it actually has, rather than
/// being forced into a caller-supplied box (which would scale its stroke with it).
pub fn natural_size(icon: LayerIcon) -> f32 {
    let text = String::from_utf8_lossy(ASSETS[icon as usize].1);
    let Some(i) = text.find("viewBox=\"") else { return 16.0 };
    let after = &text[i + "viewBox=\"".len()..];
    after.split_whitespace().nth(2).and_then(|v| v.trim_end_matches('\"').parse().ok()).unwrap_or(16.0)
}

/// Replace every embedded `stroke-width` with [`ICON_STROKE`], once, at load time.
fn normalised() -> &'static Vec<&'static [u8]> {
    static CACHE: OnceLock<Vec<&'static [u8]>> = OnceLock::new();
    CACHE.get_or_init(|| {
        ASSETS
            .iter()
            .map(|(_, bytes)| {
                Box::leak(crate::widgets::icon::normalise_stroke(bytes).into_boxed_slice()) as &'static [u8]
            })
            .collect()
    })
}

/// Draw one layer glyph in a single ink colour, at its **natural** size.
pub fn paint(icon: LayerIcon, color: gpui::Rgba) -> impl IntoElement {
    let s = natural_size(icon);
    svg().path(icon.path()).w(px(s)).h(px(s)).flex_none().text_color(Hsla::from(color))
}

/// AssetSource hook: map `layer/<stem>.svg` onto the embedded bytes, with one shared stroke.
pub fn load(path: &str) -> Option<&'static [u8]> {
    let stem = path.strip_prefix("layer/")?.strip_suffix(".svg")?;
    let index = ASSETS.iter().position(|(name, _)| *name == stem)?;
    Some(normalised()[index])
}

pub fn paths() -> Vec<SharedString> {
    ASSETS.iter().map(|(name, _)| SharedString::from(format!("layer/{name}.svg"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// No glyph may carry its own stroke width: the painter and the SVGs share one constant.
    #[test]
    fn every_stroke_uses_the_shared_constant() {
        for (stem, _) in ASSETS {
            let bytes = load(&format!("layer/{stem}.svg")).unwrap();
            let text = std::str::from_utf8(bytes).unwrap();
            let needle = "stroke-width=\"";
            let mut rest = text;
            while let Some(i) = rest.find(needle) {
                let after = &rest[i + needle.len()..];
                let end = after.find('\"').unwrap();
                assert_eq!(&after[..end], format!("{}", ICON_STROKE), "{stem} has its own stroke width");
                rest = &after[end..];
            }
        }
        assert!(natural_size(LayerIcon::Vector) > 16.0, "a 24-unit glyph keeps its own size");
        assert_eq!(natural_size(LayerIcon::Rectangle), 16.0);
    }

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
