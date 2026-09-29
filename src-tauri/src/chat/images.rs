//! Local image storage. Picked and model-generated images are copied into the app's own data folder
//! so they survive the source file moving or being deleted. Paths are stored absolute in the
//! database, exactly like the C# app.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::{Error, Result};

/// Maps an image file's extension to its mime type - used for both locally stored images and inline data.
pub fn mime_from_extension(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

pub fn extension_from_mime(mime: &str) -> &'static str {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        "image/bmp" => ".bmp",
        _ => ".bin",
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Other(e.to_string())
}

/// Copies a user-picked file into local storage, returning the stored path.
pub fn store_file(images_dir: &Path, source: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(images_dir).map_err(io)?;
    let ext = source.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    let destination = images_dir.join(format!("{}{ext}", Uuid::new_v4()));
    std::fs::copy(source, &destination).map_err(io)?;
    Ok(destination)
}

/// Saves model-generated image bytes into local storage, returning the stored path.
pub fn store_bytes(images_dir: &Path, data: &[u8], mime: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(images_dir).map_err(io)?;
    let destination = images_dir.join(format!("{}{}", Uuid::new_v4(), extension_from_mime(mime)));
    std::fs::write(&destination, data).map_err(io)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_and_extension_round_trip_for_known_types() {
        for (path, mime, ext) in [
            ("a.PNG", "image/png", ".png"),
            ("a.jpeg", "image/jpeg", ".jpg"),
            ("a.webp", "image/webp", ".webp"),
            ("a.gif", "image/gif", ".gif"),
            ("a.bmp", "image/bmp", ".bmp"),
        ] {
            assert_eq!(mime_from_extension(path), mime);
            assert_eq!(extension_from_mime(mime), ext);
        }
        assert_eq!(mime_from_extension("a.txt"), "application/octet-stream");
        assert_eq!(mime_from_extension("noext"), "application/octet-stream");
        assert_eq!(extension_from_mime("text/plain"), ".bin");
    }

    #[test]
    fn store_file_copies_into_the_images_dir_with_a_fresh_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("pic.png");
        std::fs::write(&src, [1, 2, 3]).unwrap();
        let images = dir.path().join("images");

        let stored = store_file(&images, &src).unwrap();

        assert!(stored.starts_with(&images));
        assert_eq!(stored.extension().unwrap(), "png");
        assert_ne!(stored, src);
        assert_eq!(std::fs::read(&stored).unwrap(), [1, 2, 3]);
        assert!(src.exists(), "the original is left alone");
    }

    #[test]
    fn store_bytes_names_the_file_after_the_mime_type() {
        let dir = tempfile::tempdir().unwrap();
        let stored = store_bytes(dir.path(), &[9, 9], "image/webp").unwrap();
        assert_eq!(stored.extension().unwrap(), "webp");
        assert_eq!(std::fs::read(stored).unwrap(), [9, 9]);
    }

    #[test]
    fn storing_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(store_file(dir.path(), &dir.path().join("nope.png")).is_err());
    }
}
