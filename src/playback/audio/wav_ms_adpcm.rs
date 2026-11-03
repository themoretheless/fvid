//! WAVE MS ADPCM format admission; reconstruction remains in the owned decoder.
use super::{Coding, Result, invalid, unsupported};
const STANDARD: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];
pub(super) fn parse(body: &[u8], channels: u16, block: u16, bits: u16) -> Result<Coding> {
    if !(1..=2).contains(&channels) {
        return Err(unsupported("MS ADPCM WAVE requires mono or stereo"));
    }
    if bits != 4 {
        return Err(invalid("MS ADPCM WAVE requires four-bit codes"));
    }
    if body.len() < 22 {
        return Err(invalid("MS ADPCM extension is incomplete"));
    }
    let word = |at| u16::from_le_bytes([body[at], body[at + 1]]);
    let extension = usize::from(word(16));
    let count = usize::from(word(20));
    if extension < 4 {
        return Err(invalid("MS ADPCM extension is incomplete"));
    }
    if !(7..=256).contains(&count) {
        return Err(invalid("MS ADPCM coefficient count is out of range"));
    }
    if extension < 4 + count * 4 || body.len() < 18 + extension {
        return Err(invalid("MS ADPCM coefficient table is incomplete"));
    }
    let frames = crate::codec::adpcm_decoder::block_frames(
        "adpcm_ms",
        u64::from(block),
        u64::from(channels),
        4,
    )
    .and_then(|n| u16::try_from(n).ok())
    .ok_or_else(|| invalid("MS ADPCM block geometry is invalid"))?;
    if frames != word(18) {
        return Err(invalid(
            "MS ADPCM samples-per-block disagrees with block size",
        ));
    }
    for (i, &(a, b)) in STANDARD.iter().enumerate() {
        if (word(22 + i * 4) as i16, word(24 + i * 4) as i16) != (a, b) {
            return Err(invalid("MS ADPCM standard coefficient table is invalid"));
        }
    }
    Ok(Coding::MsAdpcm {
        block_bytes: block,
        block_frames: frames,
    })
}
