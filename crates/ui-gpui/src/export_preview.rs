//! A6: the Export dialog's bitmap preview.
//!
//! Everything here is an existing engine capability: `figcraft_render::render_node` rasterises
//! the selection (or the page), `to_rgba8` converts the pixels, and the project's own
//! `canvas::render_worker::wrap_rgba` wraps them as a GPUI `RenderImage`. The dialog only displays
//! the result through the existing `img(ImageSource::Render(..))` inside one `field_row_full` —
//! no new widget, no hand-rolled layout, no new renderer.

use figcraft_engine::Session;
use gpui::RenderImage;
use std::sync::Arc;

/// A quarter-scale raster keeps the preview cheap even for a large page.
const PREVIEW_SCALE: f64 = 0.25;

/// `None` when there is no document or nothing could be drawn; the dialog then shows its
/// fallback label instead of an empty box.
pub fn make(session: &Session) -> Option<Arc<RenderImage>> {
    let ds = session.ds().ok()?;
    let id = ds.selection.first().copied().unwrap_or(ds.page);
    let pix = figcraft_render::render_node(&ds.doc, id, PREVIEW_SCALE)?;
    let (w, h) = (pix.width(), pix.height());
    if w == 0 || h == 0 {
        return None;
    }
    Some(crate::canvas::render_worker::wrap_rgba(w, h, figcraft_render::to_rgba8(&pix)))
}
