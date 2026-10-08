//! Native AV1 reference film-grain synthesis, specified by section 7.18.3.
use super::super::{av1_frame::Grain, av1_picture::Picture, av1_sequence::Color};
use crate::{Result, invalid};
include!("av1_grain_gaussian.rs");
fn round(v: i32, shift: u8) -> i32 {
    if shift == 0 {
        v
    } else {
        (v + (1 << (shift - 1))) >> shift
    }
}
fn random(state: &mut u16, bits: u8) -> usize {
    let bit = (*state ^ (*state >> 1) ^ (*state >> 3) ^ (*state >> 12)) & 1;
    *state = (*state >> 1) | (bit << 15);
    (*state >> (16 - bits)) as usize
}
fn scaling(g: &Grain, p: usize) -> [i32; 256] {
    let p = if g.chroma_from_luma { 0 } else { p };
    let n = g.point_counts[p];
    let mut lut = [0; 256];
    if n == 0 {
        return lut;
    }
    let pts = &g.points[p];
    lut[..pts[0][0] as usize].fill(pts[0][1] as i32);
    for i in 0..n - 1 {
        let x = pts[i][0] as usize;
        let dx = (pts[i + 1][0] - pts[i][0]) as i32;
        let dy = pts[i + 1][1] as i32 - pts[i][1] as i32;
        let delta = dy * ((65536 + (dx >> 1)) / dx);
        for j in 0..dx as usize {
            lut[x + j] = pts[i][1] as i32 + ((j as i32 * delta + 32768) >> 16);
        }
    }
    lut[pts[n - 1][0] as usize..].fill(pts[n - 1][1] as i32);
    lut
}
fn lookup(lut: &[i32; 256], value: i32, depth: u8) -> i32 {
    let shift = depth - 8;
    let x = (value >> shift) as usize;
    if shift == 0 || x == 255 {
        lut[x]
    } else {
        lut[x]
            + round(
                (lut[x + 1] - lut[x]) * (value - ((x as i32) << shift)),
                shift,
            )
    }
}
fn noise(g: &Grain, color: &Color) -> [[i32; 82 * 73]; 3] {
    let mut arrays = [[0; 82 * 73]; 3];
    let low = -(128 << (color.depth - 8));
    let high = -low - 1;
    let subx = usize::from(color.subsampling[0]);
    let suby = usize::from(color.subsampling[1]);
    for p in 0..if color.monochrome { 1 } else { 3 } {
        let width = if p > 0 && subx == 1 { 44 } else { 82 };
        let height = if p > 0 && suby == 1 { 38 } else { 73 };
        let active = g.point_counts[p] > 0 || p > 0 && g.chroma_from_luma;
        let mut state = g.seed ^ [0, 0xb524, 0x49d8][p];
        for y in 0..height {
            for x in 0..width {
                arrays[p][y * 82 + x] = if active {
                    round(
                        GAUSSIAN[random(&mut state, 11)],
                        12 - color.depth + g.grain_shift,
                    )
                } else {
                    0
                };
            }
        }
        if !active {
            continue;
        }
        for y in 3..height {
            for x in 3..width - 3 {
                let mut sum = 0;
                let mut pos = 0;
                for dy in -(g.ar_lag as isize)..=0 {
                    for dx in -(g.ar_lag as isize)..=g.ar_lag as isize {
                        if dy == 0 && dx == 0 {
                            break;
                        }
                        sum += arrays[p]
                            [(y as isize + dy) as usize * 82 + (x as isize + dx) as usize]
                            * g.ar[p][pos] as i32;
                        pos += 1;
                    }
                }
                if p > 0 && g.point_counts[0] > 0 {
                    let lx = ((x - 3) << subx) + 3;
                    let ly = ((y - 3) << suby) + 3;
                    let mut luma = 0;
                    for yy in 0..=suby {
                        for xx in 0..=subx {
                            luma += arrays[0][(ly + yy) * 82 + lx + xx];
                        }
                    }
                    sum += round(luma, (subx + suby) as u8) * g.ar[p][pos] as i32;
                }
                arrays[p][y * 82 + x] =
                    (arrays[p][y * 82 + x] + round(sum, g.ar_shift)).clamp(low, high);
            }
        }
    }
    arrays
}
/// Rendering always copies: reference pictures must never contain synthesized grain.
pub(super) fn render(source: &Picture, color: &Color, g: &Grain, budget: usize) -> Result<Picture> {
    let w = source.size[0] as usize;
    let h = source.size[1] as usize;
    let stride = w.div_ceil(32) * 32 + 2;
    let copy = source
        .planes
        .iter()
        .map(|p| p.samples.len() * 2)
        .sum::<usize>()
        + source.segment_ids.len()
        + source.saved_motion.len() * std::mem::size_of::<super::super::av1_picture::SavedMotion>();
    let workspace = stride
        .checked_mul(34 * 2 * 4)
        .and_then(|v| v.checked_add(100_000))
        .ok_or_else(|| invalid("AV1 film grain workspace overflow"))?;
    if copy
        .checked_add(256_000)
        .and_then(|n| n.checked_add(workspace))
        .is_none_or(|n| n > budget)
    {
        return Err(invalid("AV1 film grain exceeds memory budget"));
    }
    let arrays = noise(g, color);
    let mut out = source.clone();
    let low = -(128 << (color.depth - 8));
    let high = -low - 1;
    let maximum = (1 << color.depth) - 1;
    for p in 0..if color.monochrome { 1 } else { 3 } {
        if g.point_counts[p] == 0 && !(p > 0 && g.chroma_from_luma) {
            continue;
        }
        let sx = if p == 0 {
            0
        } else {
            usize::from(color.subsampling[0])
        };
        let sy = if p == 0 {
            0
        } else {
            usize::from(color.subsampling[1])
        };
        let pw = w.div_ceil(1 << sx);
        let ph = h.div_ceil(1 << sy);
        let stride = pw.div_ceil(32 >> sx) * (32 >> sx) + (2 >> sx);
        let rows = 34 >> sy;
        let blockh = 32 >> sy;
        let mut old = vec![0; stride * rows];
        let mut current = vec![0; stride * rows];
        let lut = scaling(g, p);
        for stripe in 0..ph.div_ceil(blockh) {
            let mut state = g.seed
                ^ (((stripe * 37 + 178) & 255) as u16) << 8
                ^ ((stripe * 173 + 105) & 255) as u16;
            for bx in (0..pw).step_by(32 >> sx) {
                let r = random(&mut state, 8);
                let ox = if sx == 1 {
                    6 + (r >> 4)
                } else {
                    9 + 2 * (r >> 4)
                };
                let oy = if sy == 1 {
                    6 + (r & 15)
                } else {
                    9 + 2 * (r & 15)
                };
                for y in 0..rows {
                    for x in 0..(34 >> sx) {
                        let mut v = arrays[p][(oy + y) * 82 + ox + x];
                        if bx > 0 && g.overlap && x < (2 >> sx) {
                            let prev = current[y * stride + bx + x];
                            let (a, b) = if sx == 1 {
                                (23, 22)
                            } else if x == 0 {
                                (27, 17)
                            } else {
                                (17, 27)
                            };
                            v = round(prev * a + v * b, 5).clamp(low, high);
                        }
                        current[y * stride + bx + x] = v;
                    }
                }
            }
            for y in 0..blockh.min(ph - stripe * blockh) {
                for x in 0..pw {
                    let mut v = current[y * stride + x];
                    if stripe > 0 && g.overlap && y < (2 >> sy) {
                        let prev = old[(blockh + y) * stride + x];
                        let (a, b) = if sy == 1 {
                            (23, 22)
                        } else if y == 0 {
                            (27, 17)
                        } else {
                            (17, 27)
                        };
                        v = round(prev * a + v * b, 5).clamp(low, high);
                    }
                    let py = stripe * blockh + y;
                    let orig = source.planes[p].samples[py * source.planes[p].width + x] as i32;
                    let merged = if p == 0 {
                        orig
                    } else {
                        let lx = x << sx;
                        let ly = py << sy;
                        let luma = &source.planes[0];
                        let average = if sx == 1 {
                            round(
                                luma.samples[ly * luma.width + lx] as i32
                                    + luma.samples[ly * luma.width + (lx + 1).min(w - 1)] as i32,
                                1,
                            )
                        } else {
                            luma.samples[ly * luma.width + lx] as i32
                        };
                        if g.chroma_from_luma {
                            average
                        } else {
                            ((average * (g.luma_mult[p - 1] as i32 - 128)
                                + orig * (g.chroma_mult[p - 1] as i32 - 128))
                                >> 6)
                                .saturating_add(
                                    (g.chroma_offset[p - 1] as i32 - 256) << (color.depth - 8),
                                )
                                .clamp(0, maximum)
                        }
                    };
                    let addition = round(lookup(&lut, merged, color.depth) * v, g.scaling_shift);
                    let min = if g.restricted_range {
                        16 << (color.depth - 8)
                    } else {
                        0
                    };
                    let max = if g.restricted_range {
                        (if p == 0 || color.matrix == 0 {
                            235
                        } else {
                            240
                        }) << (color.depth - 8)
                    } else {
                        maximum
                    };
                    let stride = out.planes[p].width;
                    out.planes[p].samples[py * stride + x] =
                        (orig + addition).clamp(min, max) as u16;
                }
            }
            std::mem::swap(&mut old, &mut current);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grain_workspace_admission_and_zero_strength_preserve_source() {
        let mut decoder = super::super::Decoder::new(16 << 20);
        let frames = decoder
            .decode_packet(include_bytes!("../../tests/fixtures/av1/sequence.obu"))
            .unwrap();
        let source = &frames[0].picture;
        let color = &frames[0].color;
        let g = Grain::default();
        let error = render(source, color, &g, 0).err().unwrap().to_string();
        assert!(error.contains("AV1 film grain exceeds memory budget"));
        let out = render(source, color, &g, 16 << 20).unwrap();
        for p in 0..3 {
            assert_eq!(out.planes[p].samples, source.planes[p].samples);
        }
    }
    #[test]
    fn random_sequence_and_lookup_boundaries() {
        let mut state = 12345;
        let first = random(&mut state, 11);
        assert_eq!(state, 38940);
        assert_eq!(first, 1216);
        let g = Grain {
            point_counts: [2, 0, 0],
            points: [
                [
                    [0, 0],
                    [255, 255],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                    [0, 0],
                ],
                [[0; 2]; 14],
                [[0; 2]; 14],
            ],
            ..Grain::default()
        };
        let lut = scaling(&g, 0);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert_eq!(lookup(&lut, 4095, 12), 255);
        assert_eq!(lookup(&lut, 8, 12), 1);
    }
}
