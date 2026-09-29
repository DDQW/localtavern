//! Minimal PNG chunk reader for extracting embedded metadata text. SillyTavern stores a
//! base64-encoded character card in a tEXt or iTXt chunk keyed `chara`/`ccv3`. The PNG chunk format
//! is simple enough to walk by hand: 4-byte big-endian length, 4-byte ASCII type, `length` bytes of
//! data, 4-byte CRC.

use crate::error::{Error, Result};

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// The decoded text of the first tEXt/iTXt chunk whose keyword matches (case-insensitive), if any.
pub fn read_text_chunk(png: &[u8], keyword: &str) -> Result<Option<String>> {
    if png.len() < 8 || png[..8] != SIGNATURE {
        return Err(Error::Other("Not a valid PNG file (missing PNG signature).".into()));
    }

    let mut offset = 8usize;
    while offset + 8 <= png.len() {
        let length = u32::from_be_bytes([png[offset], png[offset + 1], png[offset + 2], png[offset + 3]]) as usize;
        let chunk_type = &png[offset + 4..offset + 8];
        let data_start = offset + 8;

        // Truncated or corrupt file: stop scanning.
        if data_start.checked_add(length).and_then(|end| end.checked_add(4)).is_none_or(|end| end > png.len()) {
            break;
        }
        let data = &png[data_start..data_start + length];

        match chunk_type {
            b"tEXt" => {
                if let Some(text) = read_text(data, keyword) {
                    return Ok(Some(text));
                }
            }
            b"iTXt" => {
                if let Some(text) = read_international_text(data, keyword) {
                    return Ok(Some(text));
                }
            }
            b"IEND" => break,
            _ => {}
        }

        offset = data_start + length + 4;
    }
    Ok(None)
}

/// Latin-1 is a direct byte-to-char mapping.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

fn read_text(data: &[u8], keyword: &str) -> Option<String> {
    let nul = data.iter().position(|b| *b == 0)?;
    latin1(&data[..nul]).eq_ignore_ascii_case(keyword).then(|| latin1(&data[nul + 1..]))
}

fn read_international_text(data: &[u8], keyword: &str) -> Option<String> {
    let keyword_end = data.iter().position(|b| *b == 0)?;
    let chunk_keyword = latin1(&data[..keyword_end]);
    let mut index = keyword_end + 1;
    if index + 2 > data.len() {
        return None;
    }

    let compressed = data[index] != 0;
    index += 2; // compression flag + compression method

    // Language tag, then translated keyword: each NUL-terminated and both ignored.
    for _ in 0..2 {
        let end = data[index..].iter().position(|b| *b == 0)?;
        index += end + 1;
    }

    if !chunk_keyword.eq_ignore_ascii_case(keyword) || compressed {
        // Compressed iTXt is not produced by SillyTavern's exporter - not worth a zlib dependency.
        return None;
    }
    Some(String::from_utf8_lossy(&data[index..]).into_owned())
}

/// Builds minimal synthetic PNG byte streams for tests. Only the signature and requested chunks are
/// included (no real IHDR/IDAT) - fine since the reader only walks the chunk list. CRC bytes are
/// zero since nothing verifies them.
#[cfg(test)]
pub mod testing {
    pub fn build(chunks: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = super::SIGNATURE.to_vec();
        for (kind, data) in chunks {
            out.extend((data.len() as u32).to_be_bytes());
            out.extend(kind.as_bytes());
            out.extend(data);
            out.extend([0, 0, 0, 0]);
        }
        out
    }

    pub fn text_chunk(keyword: &str, text: &str) -> Vec<u8> {
        let mut data: Vec<u8> = keyword.chars().map(|c| c as u8).collect();
        data.push(0);
        data.extend(text.chars().map(|c| c as u8));
        data
    }

    /// An uncompressed iTXt chunk with empty language and translated keyword.
    pub fn itxt_chunk(keyword: &str, text: &str, compressed: bool) -> Vec<u8> {
        let mut data = keyword.as_bytes().to_vec();
        data.extend([0, u8::from(compressed), 0, 0, 0]);
        data.extend(text.as_bytes());
        data
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn reads_a_matching_text_chunk_by_keyword() {
        let png = build(&[("tEXt", text_chunk("chara", "base64payload==")), ("IEND", vec![])]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap().as_deref(), Some("base64payload=="));
    }

    #[test]
    fn keyword_match_is_case_insensitive() {
        let png = build(&[("tEXt", text_chunk("Chara", "abc"))]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap().as_deref(), Some("abc"));
    }

    #[test]
    fn returns_none_when_no_matching_chunk_exists() {
        let png = build(&[("tEXt", text_chunk("Comment", "just a comment")), ("IEND", vec![])]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap(), None);
    }

    #[test]
    fn skips_unrelated_chunks_to_find_the_right_one() {
        let png = build(&[
            ("IHDR", vec![0; 13]),
            ("tEXt", text_chunk("Comment", "nope")),
            ("tEXt", text_chunk("chara", "found-it")),
            ("IEND", vec![]),
        ]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap().as_deref(), Some("found-it"));
    }

    #[test]
    fn rejects_input_without_the_png_signature() {
        let err = read_text_chunk(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "chara").unwrap_err();
        assert!(err.to_string().contains("Not a valid PNG"));
        assert!(read_text_chunk(&[], "chara").is_err());
    }

    #[test]
    fn reads_uncompressed_international_text_as_utf8() {
        let png = build(&[("iTXt", itxt_chunk("chara", "héllo 日本", false))]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap().as_deref(), Some("héllo 日本"));
    }

    #[test]
    fn ignores_compressed_international_text() {
        let png = build(&[("iTXt", itxt_chunk("chara", "zzz", true))]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap(), None);
    }

    #[test]
    fn stops_quietly_at_a_truncated_chunk() {
        let mut png = build(&[("tEXt", text_chunk("chara", "abc"))]);
        png.truncate(png.len() - 6); // cut into the chunk
        assert_eq!(read_text_chunk(&png, "chara").unwrap(), None);

        // A chunk claiming a gigantic length must not overflow or panic.
        let mut hostile = SIGNATURE.to_vec();
        hostile.extend(u32::MAX.to_be_bytes());
        hostile.extend(b"tEXt");
        assert_eq!(read_text_chunk(&hostile, "chara").unwrap(), None);
    }

    #[test]
    fn a_chunk_without_a_separator_is_skipped() {
        let png = build(&[("tEXt", b"no separator".to_vec()), ("tEXt", text_chunk("chara", "ok"))]);
        assert_eq!(read_text_chunk(&png, "chara").unwrap().as_deref(), Some("ok"));
    }
}
