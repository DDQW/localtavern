//! Files the user picked in a dialog. On Android the picker hands back `content://` URIs rather
//! than paths, so a picked file is copied to a local scratch file first (named after what its bytes
//! look like, since the URI carries no extension), and a save goes through the content resolver.

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime};
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
use uuid::Uuid;

use crate::error::{Error, Result};

const CONTENT: &str = "content://";

/// A picked file as a local path. A scratch copy is deleted when this is dropped.
pub struct Picked {
    path: PathBuf,
    scratch: bool,
}

impl Picked {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Picked {
    fn drop(&mut self) {
        if self.scratch {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn content_uri(picked: &str) -> Result<Option<FilePath>> {
    if !picked.starts_with(CONTENT) {
        return Ok(None);
    }
    let url = picked.parse().map_err(|e| Error::Other(format!("Couldn't read {picked}: {e}")))?;
    Ok(Some(FilePath::Url(url)))
}

/// The picked file as something `std::fs` can read; content URIs are copied into `scratch_dir`.
pub fn open<R: Runtime>(app: &AppHandle<R>, scratch_dir: &Path, picked: &str) -> Result<Picked> {
    let Some(uri) = content_uri(picked)? else {
        return Ok(Picked { path: PathBuf::from(picked), scratch: false });
    };
    let bytes = app.fs().read(uri).map_err(|e| Error::Other(format!("Couldn't read the picked file: {e}")))?;
    std::fs::create_dir_all(scratch_dir).map_err(|e| Error::Other(e.to_string()))?;
    let path = scratch_dir.join(format!("{}.{}", Uuid::new_v4(), sniff_extension(&bytes)));
    std::fs::write(&path, &bytes).map_err(|e| Error::Other(e.to_string()))?;
    Ok(Picked { path, scratch: true })
}

/// Writes `data` to a path or content URI from a save dialog.
pub fn write<R: Runtime>(app: &AppHandle<R>, picked: &str, data: &[u8]) -> Result<()> {
    let fail = |e: std::io::Error| Error::Other(format!("Couldn't write {picked}: {e}"));
    let Some(uri) = content_uri(picked)? else {
        return std::fs::write(picked, data).map_err(fail);
    };
    let mut options = OpenOptions::new();
    options.write(true).truncate(true);
    app.fs().open(uri, options).and_then(|mut file| file.write_all(data)).map_err(fail)
}

/// The extension the import and image code expect for these bytes.
fn sniff_extension(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, 0xFF, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'B', b'M', ..] => "bmp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ if starts_like_json(bytes) => "json",
        _ => "bin",
    }
}

/// A JSON object, perhaps after a UTF-8 byte order mark and whitespace.
fn starts_like_json(bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_formats_the_app_imports() {
        assert_eq!(sniff_extension(b"\x89PNG\r\n\x1a\n"), "png");
        assert_eq!(sniff_extension(b"\xFF\xD8\xFF\xE0"), "jpg");
        assert_eq!(sniff_extension(b"GIF89a"), "gif");
        assert_eq!(sniff_extension(b"BM\0\0"), "bmp");
        assert_eq!(sniff_extension(b"RIFF\0\0\0\0WEBPVP8 "), "webp");
        assert_eq!(sniff_extension(b"\xEF\xBB\xBF{}"), "json");
        assert_eq!(sniff_extension(b"not a card"), "bin");
        assert_eq!(sniff_extension(b"  \n{\"spec\":\"chara_card_v2\"}"), "json");
        assert_eq!(sniff_extension(b""), "bin");
    }
}
