//! Owned Layer III scalefactor parsing, including MPEG-2 LSF intensity layouts.
use crate::{
    Result, invalid,
    mp3_frame::{Granule, Version},
    mp3_huffman::Cursor,
    mp3_hybrid::Window,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScaleFactors {
    pub long: [u8; 22],
    pub short: [[u8; 3]; 13],
    /// Transmitted widths are retained for LSF intensity's all-ones marker.
    /// A zero-width band has implicit value/position zero, not an escape marker.
    pub long_bits: [u8; 22],
    pub short_bits: [[u8; 3]; 13],
    pub preflag: bool,
    pub intensity_scale: bool,
    pub window: Window,
}
impl ScaleFactors {
    fn new(window: Window) -> Self {
        Self {
            long: [0; 22],
            short: [[0; 3]; 13],
            long_bits: [0; 22],
            short_bits: [[0; 3]; 13],
            preflag: false,
            intensity_scale: false,
            window,
        }
    }
}
const MPEG1_LENGTHS: [[u8; 2]; 16] = [
    [0, 0],
    [0, 1],
    [0, 2],
    [0, 3],
    [3, 0],
    [1, 1],
    [1, 2],
    [1, 3],
    [2, 1],
    [2, 2],
    [2, 3],
    [3, 1],
    [3, 2],
    [3, 3],
    [4, 2],
    [4, 3],
];
// ISO/IEC 13818-3: partition cardinalities for long, short, and mixed layouts.
const COUNTS: [[[u8; 4]; 3]; 6] = [
    [[6, 5, 5, 5], [9, 9, 9, 9], [6, 9, 9, 9]],
    [[6, 5, 7, 3], [9, 9, 12, 6], [6, 9, 12, 6]],
    [[11, 10, 0, 0], [18, 18, 0, 0], [15, 18, 0, 0]],
    [[7, 7, 7, 0], [12, 12, 12, 0], [6, 15, 12, 0]],
    [[6, 6, 6, 3], [12, 9, 9, 6], [6, 12, 9, 6]],
    [[8, 8, 5, 0], [15, 12, 9, 0], [6, 18, 9, 0]],
];
fn lsf(compress: u16, intensity: bool) -> Result<([u8; 4], usize, bool, bool)> {
    if compress > 511 {
        return Err(invalid("LSF scalefac_compress exceeds nine bits"));
    }
    let n = usize::from(compress);
    let (lengths, set, preflag) = if intensity {
        let n = n / 2;
        if n < 180 {
            ([n / 36, (n / 6) % 6, n % 6, 0], 3, false)
        } else if n < 244 {
            let q = n - 180;
            ([q / 16, (q / 4) % 4, q % 4, 0], 4, false)
        } else {
            let q = n - 244;
            ([q / 3, q % 3, 0, 0], 5, false)
        }
    } else if n < 400 {
        ([n / 80, (n / 16) % 5, (n / 4) % 4, n % 4], 0, false)
    } else if n < 500 {
        let q = n - 400;
        ([q / 20, (q / 4) % 5, q % 4, 0], 1, false)
    } else {
        let q = n - 500;
        ([q / 3, q % 3, 0, 0], 2, true)
    };
    Ok((
        lengths.map(|v| v as u8),
        set,
        preflag,
        intensity && compress % 2 != 0,
    ))
}
/// Read only scalefactors from a granule-bounded cursor. On failure, no bits
/// are consumed. `intensity_channel` denotes the LSF joint-stereo right channel.
pub fn read(
    cursor: &mut Cursor<'_>,
    version: Version,
    g: &Granule,
    granule_index: u8,
    scfsi: [bool; 4],
    previous: Option<&ScaleFactors>,
    intensity_channel: bool,
) -> Result<ScaleFactors> {
    if granule_index > 1 || (version != Version::Mpeg1 && granule_index != 0) {
        return Err(invalid("invalid MP3 scalefactor granule index"));
    }
    let mut next = cursor.clone();
    let mut output = ScaleFactors::new(g.window);
    if version == Version::Mpeg1 {
        let lengths = *MPEG1_LENGTHS
            .get(usize::from(g.scalefac_compress))
            .ok_or_else(|| invalid("MPEG-1 scalefac_compress exceeds four bits"))?;
        output.preflag = g.preflag.unwrap_or(false);
        if g.window == Window::Short {
            if scfsi.iter().any(|&v| v) {
                return Err(invalid("MP3 scfsi cannot be used with short windows"));
            }
            let start = if g.mixed {
                for band in 0..8 {
                    output.long[band] = next.read(usize::from(lengths[0]))? as u8;
                    output.long_bits[band] = lengths[0];
                }
                3
            } else {
                0
            };
            for band in start..12 {
                let width = lengths[usize::from(band >= 6)];
                for w in 0..3 {
                    output.short[band][w] = next.read(usize::from(width))? as u8;
                    output.short_bits[band][w] = width;
                }
            }
        } else {
            for (group, (start, end)) in [(0, 6), (6, 11), (11, 16), (16, 21)]
                .into_iter()
                .enumerate()
            {
                if granule_index == 1 && scfsi[group] {
                    let previous = previous.ok_or_else(|| {
                        invalid("MP3 scfsi requires previous granule scalefactors")
                    })?;
                    if previous.window == Window::Short {
                        return Err(invalid("MP3 scfsi cannot reuse short-window scalefactors"));
                    }
                    output.long[start..end].copy_from_slice(&previous.long[start..end]);
                    output.long_bits[start..end].copy_from_slice(&previous.long_bits[start..end]);
                } else {
                    for band in start..end {
                        let width = lengths[usize::from(band >= 11)];
                        output.long[band] = next.read(usize::from(width))? as u8;
                        output.long_bits[band] = width;
                    }
                }
            }
        }
    } else {
        if scfsi.iter().any(|&v| v) {
            return Err(invalid("LSF MP3 does not transmit scfsi"));
        }
        let (lengths, set, preflag, intensity_scale) = lsf(g.scalefac_compress, intensity_channel)?;
        output.preflag = preflag;
        output.intensity_scale = intensity_scale;
        let layout = if g.window != Window::Short {
            0
        } else if g.mixed {
            2
        } else {
            1
        };
        let mut index = 0;
        for (part, &count) in COUNTS[set][layout].iter().enumerate() {
            let width = lengths[part];
            for _ in 0..count {
                let value = next.read(usize::from(width))? as u8;
                if layout == 0 || (layout == 2 && index < 6) {
                    output.long[index] = value;
                    output.long_bits[index] = width;
                } else {
                    let (band, w) = if layout == 1 {
                        (index / 3, index % 3)
                    } else {
                        (3 + (index - 6) / 3, (index - 6) % 3)
                    };
                    output.short[band][w] = value;
                    output.short_bits[band][w] = width;
                }
                index += 1;
            }
        }
        if index != [21, 36, 33][layout] {
            return Err(invalid("invalid LSF scalefactor partition plan"));
        }
    }
    *cursor = next;
    Ok(output)
}
