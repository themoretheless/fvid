//! FLAC Vorbis-comment speaker masks and tag extraction, RFC 9639 section 8.6.2.
use fvid_control::error::{invalid, Result};
use std::collections::BTreeMap;
fn take<'a>(data: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    let (a, b) = data
        .split_at_checked(n)
        .ok_or_else(|| invalid("truncated FLAC Vorbis comment"))?;
    *data = b;
    Ok(a)
}
fn size(data: &mut &[u8]) -> Result<usize> {
    Ok(u32::from_le_bytes(take(data, 4)?.try_into().unwrap()) as usize)
}
fn parse_comment_block(mut data: &[u8]) -> Result<(Option<u32>, Vec<(String, String)>)> {
    let n = size(&mut data)?;
    let vendor = take(&mut data, n)?;
    std::str::from_utf8(vendor).map_err(|_| invalid("invalid FLAC comment UTF-8"))?;
    let count = size(&mut data)?;
    if count.saturating_mul(4) > data.len() {
        return Err(invalid("truncated FLAC Vorbis comment"));
    }
    let mut mask = None;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let n = size(&mut data)?;
        let field = std::str::from_utf8(take(&mut data, n)?)
            .map_err(|_| invalid("invalid FLAC comment UTF-8"))?;
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| invalid("invalid FLAC comment field"))?;
        if key.is_empty() || !key.bytes().all(|b| (32..=126).contains(&b) && b != b'=') {
            return Err(invalid("invalid FLAC comment field name"));
        }
        if key.eq_ignore_ascii_case("WAVEFORMATEXTENSIBLE_CHANNEL_MASK") {
            let hex = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .ok_or_else(|| invalid("invalid FLAC channel mask"))?;
            if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid("invalid FLAC channel mask"));
            }
            let value =
                u32::from_str_radix(hex, 16).map_err(|_| invalid("FLAC channel mask overflow"))?;
            if mask.is_some_and(|old| old != value) {
                return Err(invalid("conflicting FLAC channel masks"));
            }
            mask = Some(value);
        } else {
            entries.push((key.to_owned(), value.to_owned()));
        }
    }
    if !data.is_empty() {
        return Err(invalid("trailing FLAC comment bytes"));
    }
    Ok((mask, entries))
}
fn scan_blocks(extra: &[u8]) -> Result<(Option<u32>, Vec<(String, String)>)> {
    if extra.len() == 34 {
        return Ok((None, Vec::new()));
    }
    let mut data = extra.strip_prefix(b"fLaC").unwrap_or(extra);
    let mut mask = None;
    let mut entries = Vec::new();
    let mut seen = false;
    loop {
        let header = take(&mut data, 4)?;
        let n =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let body = take(&mut data, n)?;
        if header[0] & 127 == 4 {
            if seen {
                return Err(invalid("duplicate FLAC Vorbis comment block"));
            }
            seen = true;
            (mask, entries) = parse_comment_block(body)?;
        }
        if header[0] & 128 != 0 {
            return Ok((mask, entries));
        }
    }
}
/// Read a raw STREAMINFO body, or metadata block chain with optional fLaC marker.
/// Zero and partial masks retain explicitly unassigned channels.
pub fn channel_mask(extra: &[u8]) -> Result<Option<u32>> {
    Ok(scan_blocks(extra)?.0)
}
/// Read raw Vorbis comment key-value entries from a FLAC metadata block chain.
pub fn vorbis_comments(extra: &[u8]) -> Result<Vec<(String, String)>> {
    Ok(scan_blocks(extra)?.1)
}
/// Map Vorbis comment entries onto canonical probe container and stream metadata maps.
pub fn probe_metadata_from_entries<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut container = BTreeMap::new();
    let mut stream = BTreeMap::new();
    for (raw_key, value) in entries {
        if value.is_empty()
            || raw_key.eq_ignore_ascii_case("WAVEFORMATEXTENSIBLE_CHANNEL_MASK")
        {
            continue;
        }
        let lower = raw_key.to_ascii_lowercase();
        let canonical = match lower.as_str() {
            "tracknumber" | "part_number" => "track",
            "albumartist" => "album_artist",
            "discnumber" => "disc",
            other => other,
        };
        container
            .entry(canonical.to_owned())
            .or_insert_with(|| value.to_owned());
        if matches!(canonical, "title" | "language") {
            stream
                .entry(canonical.to_owned())
                .or_insert_with(|| value.to_owned());
        }
    }
    (container, stream)
}
/// Build RIFF WAVE INFO chunks from FLAC metadata blocks for WAVE spooling.
pub fn wave_info_from_extra(extra: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let comments = vorbis_comments(extra).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for (source, target) in [
        ("TITLE", "title"),
        ("ARTIST", "artist"),
        ("COMMENT", "comment"),
        ("DATE", "date"),
        ("ALBUM", "album"),
        ("GENRE", "genre"),
        ("COPYRIGHT", "copyright"),
        ("ENCODER", "encoder"),
        ("TRACKNUMBER", "track"),
        ("LANGUAGE", "ILNG"),
    ] {
        if let Some((_, value)) = comments
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(source))
        {
            if !value.contains('\0') {
                entries.push((target.to_string(), value.clone()));
            }
        }
    }
    crate::owned_wave_metadata::edit_info_chunks(&[], &[], &entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn comment(fields: &[&str]) -> Vec<u8> {
        let mut body = vec![0; 4];
        body.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        for f in fields {
            body.extend_from_slice(&(f.len() as u32).to_le_bytes());
            body.extend_from_slice(f.as_bytes());
        }
        let mut data = vec![132];
        data.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        data.extend(body);
        data
    }
    #[test]
    fn comment_names_hex_case_padding_and_unassigned_positions() {
        for (text, value) in [
            ("waveformatextensible_channel_mask=0X00000008", 8),
            ("WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0000000000000003", 3),
            ("WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0", 0),
        ] {
            assert_eq!(channel_mask(&comment(&[text])).unwrap(), Some(value));
        }
        assert_eq!(
            channel_mask(&comment(&["TITLE=author synthetic"])).unwrap(),
            None
        );
    }
    #[test]
    fn invalid_masks_lengths_and_conflicting_metadata_refuse_specifically() {
        for text in [
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=8",
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x",
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x100000000",
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0xgg",
        ] {
            assert!(channel_mask(&comment(&[text]))
                .unwrap_err()
                .to_string()
                .contains("mask"));
        }
        assert!(channel_mask(&comment(&[
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3",
            "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x8"
        ]))
        .unwrap_err()
        .to_string()
        .contains("conflicting"));
        let mut data = comment(&["TITLE=hello"]);
        data.pop();
        assert!(channel_mask(&data)
            .unwrap_err()
            .to_string()
            .contains("truncated"));
    }
}

/// RFC 9639 default channel assignment, including quad for four channels.
pub fn default_channel_mask(channels: u16) -> u32 {
    if channels == 4 {
        0x33
    } else {
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
    }
}
