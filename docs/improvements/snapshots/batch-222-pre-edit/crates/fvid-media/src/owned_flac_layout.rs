//! FLAC Vorbis-comment speaker masks, RFC 9639 section 8.6.2.
use fvid_control::error::{invalid, Result};
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
fn comments(mut data: &[u8]) -> Result<Option<u32>> {
    let n = size(&mut data)?;
    let vendor = take(&mut data, n)?;
    std::str::from_utf8(vendor).map_err(|_| invalid("invalid FLAC comment UTF-8"))?;
    let count = size(&mut data)?;
    let mut mask = None;
    for _ in 0..count {
        let n = size(&mut data)?;
        let field = std::str::from_utf8(take(&mut data, n)?)
            .map_err(|_| invalid("invalid FLAC comment UTF-8"))?;
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| invalid("invalid FLAC comment field"))?;
        if !key.bytes().all(|b| (32..=126).contains(&b) && b != b'=') {
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
        }
    }
    if !data.is_empty() {
        return Err(invalid("trailing FLAC comment bytes"));
    }
    Ok(mask)
}
/// Read a raw STREAMINFO body, or metadata block chain with optional fLaC marker.
/// Zero and partial masks retain explicitly unassigned channels.
pub fn channel_mask(extra: &[u8]) -> Result<Option<u32>> {
    if extra.len() == 34 {
        return Ok(None);
    }
    let mut data = extra.strip_prefix(b"fLaC").unwrap_or(extra);
    let mut mask = None;
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
            mask = comments(body)?;
        }
        if header[0] & 128 != 0 {
            return Ok(mask);
        }
    }
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
