//! Lossless reconstruction using lossy predictor histories and correction offsets.
use super::*;
struct Shaping {
    error: [i32; 2],
    accum: [i32; 2],
    delta: [i32; 2],
    new: bool,
}
impl Shaping {
    fn read(block: &Block<'_>) -> Result<Self> {
        let mut out = Self {
            error: [0; 2],
            accum: [0; 2],
            delta: [0; 2],
            new: block.flags & (1 << 29) != 0,
        };
        if block.flags & SHAPING == 0 || block.shaping.is_empty() {
            return Ok(out);
        }
        let data = block.shaping;
        let n = usize::from(block.coded_channels());
        if data.len() == 2 {
            for c in 0..2 {
                out.accum[c] = restore_weight(data[c]).wrapping_shl(16);
            }
            return Ok(out);
        }
        if data.len() != 4 * n && data.len() != 6 * n {
            return Err(invalid(
                "WavPack correction shaping information has wrong length",
            ));
        }
        let expand = |at| -> Result<i32> {
            let log = i32::from(le16(data, at));
            if log.abs() > 8447 {
                return Err(invalid("WavPack shaping exponent exceeds 32 bits"));
            }
            Ok(exp2_value(log))
        };
        for c in 0..n {
            out.error[c] = expand(c * 4)?;
            out.accum[c] = expand(c * 4 + 2)?;
        }
        if data.len() == 6 * n {
            for c in 0..n {
                out.delta[c] = expand(n * 4 + c * 2)?;
            }
        }
        Ok(out)
    }
    fn apply(&mut self, c: usize, lossy: i32, exact: i32) -> i32 {
        self.accum[c] = self.accum[c].wrapping_add(self.delta[c]);
        let weight = self.accum[c] >> 16;
        let mut adjustment = apply_weight(weight, self.error[c]).wrapping_neg();
        let correction = exact.wrapping_sub(lossy);
        if self.new && weight < 0 && adjustment != 0 {
            if adjustment == self.error[c] {
                adjustment = adjustment.wrapping_add(if adjustment < 0 { 1 } else { -1 });
            }
            self.error[c] = adjustment.wrapping_sub(correction);
        } else {
            self.error[c] = correction.wrapping_neg();
        }
        exact.wrapping_sub(adjustment)
    }
}
pub(super) fn reconstruct(
    block: &Block<'_>,
    residuals: &[i32],
    offsets: &[i32],
) -> Result<Vec<i32>> {
    if residuals.len() != offsets.len() {
        return Err(invalid("WavPack correction sample count mismatch"));
    }
    let n = usize::from(block.coded_channels());
    let cross = block.flags & 32 != 0 && n == 2;
    let mut passes = block.passes()?;
    let mut shaping = Shaping::read(block)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(residuals.len())
        .map_err(|_| invalid("cannot allocate WavPack corrected samples"))?;
    for (row, deltas) in residuals.chunks_exact(n).zip(offsets.chunks_exact(n)) {
        let mut lossy = [row[0], if n == 2 { row[1] } else { 0 }];
        let mut exact = [
            lossy[0].wrapping_add(deltas[0]),
            if n == 2 {
                lossy[1].wrapping_add(deltas[1])
            } else {
                0
            },
        ];
        for pass in &mut passes {
            if cross {
                let mut snapshot = *pass;
                stereo_pass(&mut snapshot, &mut exact);
            }
            if n == 2 {
                stereo_pass(pass, &mut lossy);
            } else {
                mono_pass(pass, &mut lossy[..1]);
            }
        }
        if !cross {
            for c in 0..n {
                exact[c] = lossy[c].wrapping_add(deltas[c]);
            }
        }
        if n == 2 && block.flags & JOINT_STEREO != 0 {
            for pair in [&mut lossy, &mut exact] {
                pair[1] = pair[1].wrapping_sub(pair[0] >> 1);
                pair[0] = pair[0].wrapping_add(pair[1]);
            }
        }
        for c in 0..n {
            output.push(if block.flags & SHAPING != 0 {
                shaping.apply(c, lossy[c], exact[c])
            } else {
                exact[c]
            });
        }
    }
    Ok(output)
}

/// Join corresponding full-header main/correction components into owned packets.
/// The corrected CRC replaces the lossy CRC; no PCM is published before it passes.
pub(super) fn join(main: &[u8], extra: &[u8], channels: u16) -> Result<Vec<u8>> {
    let frame = frame::Frame::parse(main, channels)?;
    let mut m = 0usize;
    let mut c = 0usize;
    let mut out = Vec::new();
    for block in &frame.blocks {
        let head = extra
            .get(
                c..c.checked_add(32)
                    .ok_or_else(|| invalid("WavPack correction header overflow"))?,
            )
            .ok_or_else(|| invalid("truncated WavPack correction header"))?;
        if &head[..4] != b"wvpk" || head[8..28] != main[m + 8..m + 28] {
            return Err(invalid(
                "WavPack correction header disagrees with main block",
            ));
        }
        if block.flags & HYBRID == 0 {
            return Err(invalid("WavPack correction requires hybrid main stream"));
        }
        let size = (le32(head, 4) as usize)
            .checked_add(8)
            .filter(|&n| n >= 32)
            .ok_or_else(|| invalid("invalid WavPack correction size"))?;
        let end = c
            .checked_add(size)
            .ok_or_else(|| invalid("WavPack correction size overflow"))?;
        let body = extra
            .get(c + 32..end)
            .ok_or_else(|| invalid("WavPack correction exceeds packet"))?;
        let mut at = 0;
        let mut has_bits = false;
        while at < body.len() {
            if body.len() - at < 2 {
                return Err(invalid("truncated WavPack correction metadata"));
            }
            let tag = body[at];
            has_bits |= tag & ID_MASK == ID_CORRECTION;
            let h = if tag & 0x80 != 0 { 4 } else { 2 };
            if body.len() - at < h {
                return Err(invalid("truncated WavPack correction metadata"));
            }
            let words = if h == 4 {
                le24(&body[at + 1..]) as usize
            } else {
                body[at + 1] as usize
            };
            if !matches!(
                tag & ID_MASK,
                ID_CORRECTION | ID_SHAPING_WEIGHTS | ID_EXTRA_BITS | 0x2c
            ) && tag & IDF_IGNORE == 0
            {
                return Err(invalid("unsupported metadata in WavPack correction block"));
            }
            at = at
                .checked_add(h + words * 2)
                .filter(|&n| n <= body.len())
                .ok_or_else(|| invalid("WavPack correction metadata exceeds block"))?;
        }
        let length = block
            .length
            .checked_add(body.len() + if has_bits { 0 } else { 2 })
            .ok_or_else(|| invalid("WavPack combined block size overflow"))?;
        let size =
            u32::try_from(length - 8).map_err(|_| invalid("WavPack combined block too large"))?;
        out.try_reserve(length)
            .map_err(|_| invalid("cannot allocate WavPack combined packet"))?;
        let start = out.len();
        out.extend_from_slice(&main[m..m + block.length]);
        out.extend_from_slice(body);
        // A correction block with no entropy bits still activates corrected
        // shaping/CRC semantics (all residual error limits may be zero).
        if !has_bits {
            out.extend_from_slice(&[ID_CORRECTION, 0]);
        }
        out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
        out[start + 28..start + 32].copy_from_slice(&head[28..32]);
        m += block.length;
        c = end;
    }
    if c != extra.len() {
        return Err(invalid("WavPack correction has trailing components"));
    }
    Ok(out)
}

pub(super) fn matroska(main: &[u8], extra: &[u8], version: u16, channels: u16) -> Result<Vec<u8>> {
    let raw = frame::restore_matroska(main, version, channels)?;
    let frame = frame::Frame::parse(&raw, channels)?;
    let mut correction = Vec::new();
    let mut at = 0usize;
    let mut m = 0usize;
    for block in &frame.blocks {
        let n = if channels > 2 { 8 } else { 4 };
        if at.checked_add(n).is_none_or(|n| n > extra.len()) {
            return Err(invalid("truncated Matroska WavPack correction header"));
        }
        let length = if channels > 2 {
            le32(extra, at + 4) as usize
        } else {
            extra.len() - at - n
        };
        let end = at
            .checked_add(n)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= extra.len())
            .ok_or_else(|| invalid("Matroska WavPack correction exceeds packet"))?;
        let size = u32::try_from(
            length
                .checked_add(24)
                .ok_or_else(|| invalid("WavPack correction size overflow"))?,
        )
        .map_err(|_| invalid("WavPack correction too large"))?;
        correction
            .try_reserve(length + 32)
            .map_err(|_| invalid("cannot allocate WavPack correction"))?;
        correction.extend_from_slice(&raw[m..m + 4]);
        correction.extend_from_slice(&size.to_le_bytes());
        correction.extend_from_slice(&raw[m + 8..m + 28]);
        correction.extend_from_slice(&extra[at..at + 4]);
        correction.extend_from_slice(&extra[at + n..end]);
        m += block.length;
        at = end;
    }
    if at != extra.len() {
        return Err(invalid("Matroska WavPack correction has trailing bytes"));
    }
    let joined = join(&raw, &correction, channels)?;
    let frame = frame::Frame::parse(&joined, channels)?;
    let mut out = Vec::new();
    let mut at = 0;
    out.try_reserve(joined.len())
        .map_err(|_| invalid("cannot allocate compact WavPack correction packet"))?;
    out.extend_from_slice(&joined[20..24]);
    for block in frame.blocks {
        out.extend_from_slice(&joined[at + 24..at + 32]);
        if channels > 2 {
            out.extend_from_slice(&((block.length - 32) as u32).to_le_bytes());
        }
        out.extend_from_slice(&joined[at + 32..at + block.length]);
        at += block.length;
    }
    Ok(out)
}
