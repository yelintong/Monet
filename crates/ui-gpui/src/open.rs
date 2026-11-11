//! Which files File ▸ Open accepts, and what a drop does where it lands — the port of
//! `ui-egui/src/open.rs`.
//!
//! The engine already does the work (`file.open`, `file.import`, `file.placeImage` all take a
//! `path`, or `data` + `name` when a platform has no paths); this module only decides **which**
//! command a given file goes to.

use std::path::Path;

/// The file types File ▸ Open and a drop on the Home screen open (`OPEN_EXTENSIONS`).
pub(crate) const OPEN_EXTENSIONS: &[&str] =
    &["figcraft", "fig", "jam", "penpot", "sketch", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"];

/// The image types File ▸ Place image takes (`import_dialog(true)`).
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"];

/// What File ▸ Import takes (`import_dialog(false)`): documents as pages, SVG as a frame, images
/// as a placed image.
pub(crate) const IMPORT_EXTENSIONS: &[&str] =
    &["penpot", "svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff", "figcraft", "fig", "jam", "sketch"];

/// The types that are a whole document rather than a picture: dropped on the canvas they open as a
/// file of their own instead of adding their pages to the open one (`is_document`).
const DOCUMENT_EXTENSIONS: &[&str] = &["figcraft", "fig", "jam", "penpot", "sketch", "json"];

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` names a file type that opens as a document (the extension, in any case).
pub(crate) fn opens(path: &Path) -> bool {
    has_ext(path, OPEN_EXTENSIONS)
}

/// Whether `path` is a whole document (`.figcraft`, `.fig`, `.jam`, `.penpot`, `.sketch`, file
/// JSON) rather than an image or SVG.
pub(crate) fn is_document(path: &Path) -> bool {
    has_ext(path, DOCUMENT_EXTENSIONS)
}

/// Whether `path` is an image File ▸ Place image can take.
pub(crate) fn is_image(path: &Path) -> bool {
    has_ext(path, IMAGE_EXTENSIONS)
}

/// The file's name as a drop reports it (`dropped.png` when the path has none).
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "dropped.png".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document opens as its own file; an image or SVG is imported into the open one.
    #[test]
    fn documents_and_pictures_are_told_apart() {
        for name in ["a.figcraft", "a.fig", "a.jam", "a.penpot", "a.sketch", "a.json"] {
            let p = Path::new(name);
            assert!(opens(p), "{name} opens");
            assert!(is_document(p), "{name} is a document");
            assert!(!is_image(p), "{name} is not an image");
        }
        for name in ["a.png", "a.JPG", "a.jpeg", "a.webp", "a.gif", "a.tif", "a.tiff"] {
            let p = Path::new(name);
            assert!(opens(p), "{name} opens");
            assert!(is_image(p), "{name} is an image");
            assert!(!is_document(p), "{name} is not a document");
        }
        // SVG opens and imports, but is not a document and not an image: it becomes a frame.
        let svg = Path::new("a.svg");
        assert!(opens(svg) && !is_document(svg) && !is_image(svg));
        // Anything else is turned away.
        assert!(!opens(Path::new("a.txt")));
        assert!(!opens(Path::new("a")));
        assert_eq!(file_name(Path::new("/tmp/x/y.png")), "y.png");
    }
}
