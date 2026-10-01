//! Opus transport metadata only; this module does not decode audio samples.
//! Framing: RFC 6716 section 3; identification: RFC 7845 section 5.1.
use crate::{Result, invalid};

pub fn header_channels(header: &[u8]) -> Result<u8> {
    if header.len() != 19
        || &header[..8] != b"OpusHead"
        || header[8] != 1
        || !matches!(header[9], 1 | 2)
        || header[18] != 0
    {
        return Err(invalid(
            "owned Opus transport requires version 1 mono/stereo mapping family 0",
        ));
    }
    Ok(header[9])
}

pub fn pre_skip_ns(header: &[u8]) -> Result<u64> {
    header_channels(header)?;
    Ok(u64::from(u16::from_le_bytes([header[10], header[11]])) * 1_000_000_000 / 48_000)
}

/// Validate packet framing and return its decoded duration, without inspecting entropy data.
pub fn duration_ns(packet: &[u8]) -> Result<u64> {
    let toc = *packet.first().ok_or_else(|| invalid("empty Opus packet"))?;
    let config = toc >> 3;
    let frame_us = if config < 12 {
        [10_000, 20_000, 40_000, 60_000][usize::from(config & 3)]
    } else if config < 16 {
        [10_000, 20_000][usize::from(config & 1)]
    } else {
        [2_500, 5_000, 10_000, 20_000][usize::from(config & 3)]
    };
    let mut offset = 1;
    let mut end = packet.len();
    let code = toc & 3;
    let (count, vbr) = match code {
        0 => (1, false),
        1 => (2, false),
        2 => (2, true),
        _ => {
            let control = *packet
                .get(offset)
                .ok_or_else(|| invalid("missing Opus frame count"))?;
            offset += 1;
            if control & 64 != 0 {
                let mut padding = 0usize;
                loop {
                    let byte = *packet
                        .get(offset)
                        .ok_or_else(|| invalid("truncated Opus padding"))?;
                    offset += 1;
                    padding = padding
                        .checked_add(if byte == 255 { 254 } else { usize::from(byte) })
                        .ok_or_else(|| invalid("Opus padding overflow"))?;
                    if byte != 255 {
                        break;
                    }
                }
                end = end
                    .checked_sub(padding)
                    .filter(|&n| n >= offset)
                    .ok_or_else(|| invalid("Opus padding exceeds packet"))?;
            }
            (usize::from(control & 63), control & 128 != 0)
        }
    };
    if count == 0 || frame_us * count > 120_000 {
        return Err(invalid("invalid Opus packet duration"));
    }
    if vbr {
        let mut sizes = 0usize;
        for _ in 1..count {
            let first = *packet
                .get(offset)
                .filter(|_| offset < end)
                .ok_or_else(|| invalid("truncated Opus frame length"))?;
            offset += 1;
            let size = if first < 252 {
                usize::from(first)
            } else {
                let second = *packet
                    .get(offset)
                    .filter(|_| offset < end)
                    .ok_or_else(|| invalid("truncated Opus frame length"))?;
                offset += 1;
                usize::from(first) + 4 * usize::from(second)
            };
            sizes += size;
        }
        if sizes > end.saturating_sub(offset) || end - offset - sizes > 1275 {
            return Err(invalid("invalid Opus frame lengths"));
        }
    } else if (end - offset) % count != 0 || (end - offset) / count > 1275 {
        return Err(invalid("invalid Opus CBR frame lengths"));
    }
    Ok((frame_us * count) as u64 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_toc_durations_and_frame_count_limits() {
        for config in 0..32u8 {
            let expected = if config < 12 {
                [10, 20, 40, 60][usize::from(config & 3)] * 1_000_000
            } else if config < 16 {
                [10, 20][usize::from(config & 1)] * 1_000_000
            } else {
                [2500, 5000, 10000, 20000][usize::from(config & 3)] * 1000
            };
            assert_eq!(duration_ns(&[config << 3]).unwrap(), expected);
            assert_eq!(duration_ns(&[(config << 3) | 1]).unwrap(), expected * 2);
        }
        assert_eq!(duration_ns(&[(16 << 3) | 3, 48]).unwrap(), 120_000_000);
        for bad in [
            &[][..],
            &[3],
            &[3, 0],
            &[3, 13],
            &[1, 0],
            &[2, 5],
            &[3, 0x41, 255],
            &[3, 0x41, 10],
            &[3, 0x82, 252],
        ] {
            assert!(duration_ns(bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn vbr_padding_and_short_packets_are_checked() {
        assert_eq!(duration_ns(&[3, 0x42, 1, 9, 10, 0]).unwrap(), 20_000_000);
        assert_eq!(duration_ns(&[3, 0x82, 1, 9, 10, 11]).unwrap(), 20_000_000);
        let mut extended = vec![2, 252, 0];
        extended.extend(vec![0; 252]);
        assert_eq!(duration_ns(&extended).unwrap(), 20_000_000);
        for first in 0..=255u8 {
            for second in 0..=255u8 {
                let _ = duration_ns(&[first, second]);
            }
        }
    }
    #[test]
    fn identification_and_preskip() {
        let mut header = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec();
        assert_eq!(header_channels(&header).unwrap(), 2);
        assert_eq!(pre_skip_ns(&header).unwrap(), 6_500_000);
        header[18] = 1;
        assert!(header_channels(&header).is_err());
    }
}
