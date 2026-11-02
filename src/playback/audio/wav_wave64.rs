//! Sony Wave64: GUID chunks with header-inclusive sizes and eight-byte alignment.
use super::{Limits, Wav, parse_fmt};
use crate::{Result, invalid, unsupported};
const RIFF: [u8; 16] = [
    0x72, 0x69, 0x66, 0x66, 0x2e, 0x91, 0xcf, 0x11, 0xa5, 0xd6, 0x28, 0xdb, 0x04, 0xc1, 0x00, 0x00,
];
const TAIL: [u8; 12] = [
    0xf3, 0xac, 0xd3, 0x11, 0x8c, 0xd1, 0x00, 0xc0, 0x4f, 0x8e, 0xdb, 0x8a,
];
fn tag(id: &[u8], name: &[u8; 4]) -> bool {
    &id[..4] == name && id[4..] == TAIL
}
fn number(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight-byte field"))
}
pub(super) fn parse(bytes: &[u8], limits: &Limits) -> Result<Wav> {
    if bytes.len() < 40 || bytes[..16] != RIFF || !tag(&bytes[24..40], b"wave") {
        return Err(invalid("invalid Wave64 GUID header"));
    }
    let total = number(&bytes[16..24]);
    if total < 40 {
        return Err(invalid("Wave64 file size is shorter than its header"));
    }
    if total > limits.file_bytes as u64 {
        return Err(invalid("Wave64 file limit exceeded"));
    }
    let bound = total.min(bytes.len() as u64) as usize;
    let mut at = 40usize;
    let mut format = None;
    let mut fmt_body: Option<&[u8]> = None;
    let mut data = None;
    let mut fact = None;
    while at < bound {
        if bound - at < 24 {
            return Err(invalid("Wave64 chunk header is truncated"));
        }
        let id = &bytes[at..at + 16];
        let size = number(&bytes[at + 16..at + 24]);
        if size < 24 {
            return Err(invalid("Wave64 chunk size is shorter than its header"));
        }
        let end = (at as u64)
            .checked_add(size)
            .ok_or_else(|| invalid("Wave64 chunk extent overflow"))?;
        if end > total {
            return Err(invalid("Wave64 chunk extent exceeds file extent"));
        }
        let start = at + 24;
        let audio = tag(id, b"data");
        if !audio && end > bound as u64 {
            return Err(invalid("Wave64 metadata chunk is truncated"));
        }
        let body = &bytes[start..end.min(bound as u64) as usize];
        if audio {
            if data.is_some() {
                return Err(unsupported("multiple Wave64 data chunks are not supported"));
            }
            let declared = usize::try_from(size - 24)
                .map_err(|_| invalid("Wave64 data exceeds address space"))?;
            if declared > limits.data_bytes {
                return Err(invalid("Wave64 data limit exceeded"));
            }
            data = Some((body, declared));
        } else if tag(id, b"fmt ") {
            if let Some(first) = fmt_body {
                if first != body {
                    return Err(invalid("conflicting Wave64 fmt chunks"));
                }
            }
            if format.is_none() {
                format = Some(parse_fmt(body, limits)?);
                fmt_body = Some(body);
            }
        } else if tag(id, b"fact") {
            let count = if body.len() >= 8 {
                number(&body[..8])
            } else if body.len() == 4 {
                u64::from(u32::from_le_bytes(body.try_into().unwrap()))
            } else {
                return Err(invalid("Wave64 fact chunk is truncated"));
            };
            let count = usize::try_from(count)
                .map_err(|_| invalid("Wave64 sample count exceeds address space"))?;
            if fact.is_some_and(|first| first != count) {
                return Err(invalid("conflicting Wave64 fact sample counts"));
            }
            fact = Some(count);
        }
        let next = end
            .checked_add(7)
            .ok_or_else(|| invalid("Wave64 padding extent overflow"))?
            & !7;
        at = usize::try_from(next)
            .map_err(|_| invalid("Wave64 chunk extent exceeds address space"))?;
    }
    let format = format.ok_or_else(|| invalid("Wave64 file has no fmt chunk"))?;
    let (samples, declared) = data.ok_or_else(|| invalid("Wave64 file has no data chunk"))?;
    Wav::from_samples(
        samples,
        format,
        declared,
        fact,
        false,
        Some((total, bytes.len(), "Wave64")),
        limits,
    )
}
