//! Owned Layer III requantization and short-window frequency reordering.
//! Band widths are numerical MPEG tables (ISO 11172-3 B.8 / 13818-3),
//! including the de facto MPEG-2.5 extension. No external codec is linked.
use crate::{
    Result, invalid, mp3_frame::Granule, mp3_hybrid::Window, mp3_scalefactors::ScaleFactors,
};
const RATES: [u32; 9] = [44100, 48000, 32000, 22050, 24000, 16000, 11025, 12000, 8000];
const LONG: [[u16; 22]; 9] = [
    [
        4, 4, 4, 4, 4, 4, 6, 6, 8, 8, 10, 12, 16, 20, 24, 28, 34, 42, 50, 54, 76, 158,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 10, 12, 16, 18, 22, 28, 34, 40, 46, 54, 54, 192,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 8, 10, 12, 16, 20, 24, 30, 38, 46, 56, 68, 84, 102, 26,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 18, 22, 26, 32, 38, 46, 54, 62, 70, 76, 36,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54,
    ],
    [
        12, 12, 12, 12, 12, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 76, 90, 2, 2, 2, 2, 2,
    ],
];
const SHORT: [[u16; 13]; 9] = [
    [4, 4, 4, 4, 6, 8, 10, 12, 14, 18, 22, 30, 56],
    [4, 4, 4, 4, 6, 6, 10, 12, 14, 16, 20, 26, 66],
    [4, 4, 4, 4, 6, 8, 12, 16, 20, 26, 34, 42, 12],
    [4, 4, 4, 6, 6, 8, 10, 14, 18, 26, 32, 42, 18],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 32, 44, 12],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 30, 40, 18],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 30, 40, 18],
    [4, 4, 4, 6, 8, 10, 12, 14, 18, 24, 30, 40, 18],
    [8, 8, 8, 12, 16, 20, 24, 28, 36, 2, 2, 2, 26],
];
const PRE: [u8; 22] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2, 0,
];

#[derive(Clone, Debug)]
pub struct Bands {
    pub long: [usize; 23],
    pub short: [usize; 14],
    pub mixed_long_bands: usize,
}
impl Bands {
    pub fn for_rate(rate: u32) -> Result<Self> {
        let index = RATES
            .iter()
            .position(|&r| r == rate)
            .ok_or_else(|| invalid("invalid Layer III sample rate"))?;
        let mut long = [0; 23];
        let mut short = [0; 14];
        for i in 0..22 {
            long[i + 1] = long[i] + usize::from(LONG[index][i]);
        }
        for i in 0..13 {
            short[i + 1] = short[i] + usize::from(SHORT[index][i]);
        }
        Ok(Self {
            long,
            short,
            mixed_long_bands: if rate >= 32000 { 8 } else { 6 },
        })
    }
    pub fn mixed_subbands(&self) -> u8 {
        (self.long[self.mixed_long_bands] / 18) as u8
    }
}

fn value(q: i32, exponent_quarters: i32) -> f64 {
    if q == 0 {
        return 0.0;
    }
    let magnitude = f64::from(q.unsigned_abs());
    // x^(4/3) = x*cbrt(x); exponent is exact in quarter steps.
    (magnitude * magnitude.cbrt() * 2.0f64.powf(f64::from(exponent_quarters) / 4.0))
        .copysign(f64::from(q))
}

/// Convert Huffman order to hybrid order (3*short-frequency+window).
/// Returns a fresh array; failures cannot partially modify caller storage.
pub fn reconstruct(
    input: &[i32; 576],
    rate: u32,
    g: &Granule,
    sf: &ScaleFactors,
) -> Result<[f64; 576]> {
    let bands = Bands::for_rate(rate)?;
    if sf.window != g.window
        || g.subblock_gain.iter().any(|&v| v > 7)
        || sf
            .long
            .iter()
            .chain(sf.short.iter().flatten())
            .any(|&v| v > 63)
        || input.iter().any(|v| v.unsigned_abs() > 8206)
    {
        return Err(invalid("invalid Layer III spectral parameters"));
    }
    let mut out = [0.0; 576];
    let multiplier = if g.scalefac_scale { 4 } else { 2 };
    let gain = i32::from(g.global_gain) - 210;
    let long_count = if g.window != Window::Short {
        22
    } else if g.mixed {
        bands.mixed_long_bands
    } else {
        0
    };
    for band in 0..long_count {
        let factor = i32::from(sf.long[band]) + if sf.preflag { i32::from(PRE[band]) } else { 0 };
        let exponent = gain - multiplier * factor;
        for i in bands.long[band]..bands.long[band + 1] {
            out[i] = value(input[i], exponent);
        }
    }
    if g.window == Window::Short {
        let mut source = bands.long[long_count];
        for band in (if g.mixed { 3 } else { 0 })..13 {
            for window in 0..3 {
                let exponent = gain
                    - 8 * i32::from(g.subblock_gain[window])
                    - multiplier * i32::from(sf.short[band][window]);
                for frequency in bands.short[band]..bands.short[band + 1] {
                    out[3 * frequency + window] = value(input[source], exponent);
                    source += 1;
                }
            }
        }
        if source != 576 {
            return Err(invalid("invalid Layer III spectral layout"));
        }
    }
    Ok(out)
}
