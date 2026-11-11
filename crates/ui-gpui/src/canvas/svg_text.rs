//! 旋转文本（标尺左侧数字）。
//!
//! GPUI 0.2.2 的文字管线（`ShapedLine` / `Window::paint_glyph`）把变换矩阵写死成
//! `TransformationMatrix::unit()`，**没有** egui `TextShape::with_angle_and_anchor(-π/2)`
//! 的对应能力。整个框架里唯一带变换的公开绘制路径是 SVG：`Svg::with_transformation`
//! → `Window::paint_svg(bounds, path, transformation, color, cx)` → `MonochromeSprite`。
//! `gpui-kit` 的 `Icon::rotate()` / `.transform(Transformation::rotate(..))` 走的也是这条路，
//! 而且 kit 明确写了「只有 SVG 绘图会变换，布局/命中盒不转」——正好是标尺标签需要的语义。
//!
//! 于是把短标签动态做成一张内联 SVG（`<text>` 由 usvg 转成轮廓，渲染器已
//! `load_system_fonts()`），再用 `with_transformation` 绕自身中心旋转 -90°。
//! SVG 被渲染成 alpha mask、颜色由 `.text_color()` 注入，所以 SVG 里只用黑色。
//!
//! 资源路径把文字、字号、框尺寸全部编进去（hex），`AssetSource::load` 现场生成字节，
//! 不需要全局可变表，也不会与冻结的 `IconAssets` 冲突：非 `text:` 前缀原样委托。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// 前缀。完整格式：`text:{w}x{h}@{pt}:{hex}.svg`。
const PREFIX: &str = "text:";

/// 注册到 `Application::with_assets` 的资源源：动态文本 SVG + 原有图标。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(spec) = path.strip_prefix(PREFIX) {
            return Ok(svg_bytes(spec).map(|svg| Cow::Owned(svg.into_bytes())));
        }
        // 排版对话框字形（照 lizard typ-*.svg 1:1 移植），编译期内联、不碰冻结的 Icon 枚举。
        if let Some(name) = path.strip_prefix(crate::widgets::typ_icons::ASSET_PREFIX).and_then(|s| s.strip_suffix(".svg")) {
            return Ok(crate::widgets::typ_icons::TypIcon::load(name).map(|s| Cow::Borrowed(s.as_bytes())));
        }
        crate::widgets::icon::IconAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = crate::widgets::icon::IconAssets.list(path)?;
        all.extend(crate::widgets::typ_icons::TypIcon::list());
        Ok(all)
    }
}

/// 一个旋转标签的 SVG 资源路径。`w`/`h` 是**旋转前**的水平文字框，
/// 必须与 `Svg` 元素的布局尺寸一致，否则 `Transformation::rotate` 的绕点会偏。
pub fn text_key(text: &str, pt: f32, w: f32, h: f32) -> SharedString {
    let mut hex = String::with_capacity(text.len() * 2);
    for b in text.as_bytes() {
        hex.push_str(&format!("{b:02x}"));
    }
    SharedString::from(format!("{PREFIX}{w:.2}x{h:.2}@{pt:.2}:{hex}.svg"))
}

fn svg_bytes(spec: &str) -> Option<String> {
    let spec = spec.strip_suffix(".svg").unwrap_or(spec);
    let (geom, hex) = spec.rsplit_once(':')?;
    let (wh, pt) = geom.rsplit_once('@')?;
    let (w, h) = wh.split_once('x')?;
    let w: f32 = w.parse().ok()?;
    let h: f32 = h.parse().ok()?;
    let pt: f32 = pt.parse().ok()?;
    if !(w > 0.0 && h > 0.0 && pt > 0.0) {
        return None;
    }
    let mut text = String::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        text.push(char::from_u32(hi * 16 + lo)?);
    }
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    // 与画布 horizontal label 的排法对齐：水平居中、基线落在行高的 ~0.78 处
    // （`text_size()` 的行高是 1.25×字号），整块绕中心旋转 -90° 后正好贴在左标尺。
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><text x="{cx}" y="{baseline}" text-anchor="middle" font-family="Segoe UI, Arial, sans-serif" font-size="{pt}" fill="black">{escaped}</text></svg>"##,
        w = w,
        h = h,
        cx = w / 2.0,
        baseline = h * 0.78,
        pt = pt,
    ))
}

#[cfg(test)]
mod tests {
    use super::{svg_bytes, text_key};

    #[test]
    fn a_key_round_trips_into_an_svg_with_the_same_text() {
        let key = text_key("-150", 9.0, 20.0, 11.25);
        let svg = svg_bytes(key.strip_prefix("text:").unwrap()).unwrap();
        assert!(svg.contains(">-150</text>"), "{svg}");
        assert!(svg.contains("width=\"20\""), "{svg}");
        assert!(svg.contains("font-size=\"9\""), "{svg}");
        assert!(svg.contains("text-anchor=\"middle\""), "{svg}");
    }

    #[test]
    fn malformed_keys_do_not_panic() {
        assert!(svg_bytes("garbage").is_none());
        assert!(svg_bytes("20x11@9:zz.svg").is_none());
    }
}
