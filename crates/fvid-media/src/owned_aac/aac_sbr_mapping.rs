//! Owned SBR dequantized parameter/harmonic mapping and frame history.
use super::{
    Result,
    aac_sbr_bands::FrequencyTables,
    aac_sbr_gain::Band,
    aac_sbr_grid::{FrameClass, GridSyntax},
    bits::BitReader,
    invalid,
};
/// Parse sbr_sinusoidal_coding, with the enclosing channel's harmonic flag.
/// An absent flag clears every line and consumes no harmonic payload bits.
pub fn read_harmonics(
    bits: &mut BitReader<'_>,
    end: usize,
    present: bool,
    count: usize,
) -> Result<Vec<bool>> {
    if !(1..=64).contains(&count)
        || end < bits.position()
        || end - bits.position() > bits.remaining()
        || (present && end - bits.position() < count)
    {
        return Err(invalid("invalid or truncated SBR harmonic payload"));
    }
    if !present {
        return Ok(vec![false; count]);
    }
    let mut trial = bits.clone();
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(trial.read(1)? != 0);
    }
    *bits = trial;
    Ok(result)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    lines: [bool; 64],
    carry_attack: bool,
}
impl Default for History {
    fn default() -> Self {
        Self {
            lines: [false; 64],
            carry_attack: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Mapped {
    pub bands: Vec<Vec<Band>>,
    /// l_A may equal the envelope count, deferring the attack to next frame.
    pub attack: Option<usize>,
    pub suppress_noise: Vec<bool>,
}
fn borders(b: &[u8]) -> bool {
    b.len() >= 2
        && b.len() <= 65
        && b[0] > 0
        && b[b.len() - 1] <= 64
        && b.windows(2).all(|x| x[0] < x[1])
}
impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Map already dequantized scalefactors and estimated current energy.
    /// History stores last-envelope harmonic lines at absolute QMF indices,
    /// not high-band indices, so changed frequency ranges cannot inherit an
    /// unrelated line. Validate the whole frame before committing history.
    pub fn map(
        &mut self,
        syntax: &GridSyntax,
        slots: u8,
        tables: &FrequencyTables,
        envelope: &[Vec<f64>],
        noise: &[Vec<f64>],
        harmonics: &[bool],
        current: &[Vec<f64>],
    ) -> Result<Mapped> {
        let time = syntax.time_grid(slots)?;
        let count = syntax.high_resolution.len();
        if !borders(&tables.high)
            || !borders(&tables.low)
            || !borders(&tables.noise)
            || tables.noise.len() > 6
            || tables.high.first() != tables.low.first()
            || tables.high.last() != tables.low.last()
            || tables.high.first() != tables.noise.first()
            || tables.high.last() != tables.noise.last()
            || tables.low.iter().any(|x| !tables.high.contains(x))
            || tables.noise.iter().any(|x| !tables.low.contains(x))
            || envelope.len() != count
            || current.len() != count
            || noise.len() + 1 != time.noise.len()
            || harmonics.len() + 1 != tables.high.len()
        {
            return Err(invalid("invalid SBR mapping geometry"));
        }
        let kx = usize::from(tables.high[0]);
        let stop = usize::from(*tables.high.last().unwrap());
        for (l, &fine) in syntax.high_resolution.iter().enumerate() {
            let frequency = if fine { &tables.high } else { &tables.low };
            if envelope[l].len() + 1 != frequency.len() || current[l].len() != stop - kx {
                return Err(invalid("SBR mapping row does not match frequency bands"));
            }
        }
        if noise.iter().any(|r| r.len() + 1 != tables.noise.len())
            || envelope
                .iter()
                .chain(noise)
                .chain(current)
                .flatten()
                .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err(invalid("invalid SBR mapping scalefactors or energies"));
        }
        let pointer = usize::from(syntax.pointer);
        let attack = match syntax.class {
            FrameClass::FixFix => None,
            FrameClass::FixVar | FrameClass::VarVar => {
                if pointer == 0 {
                    None
                } else {
                    Some(count + 1 - pointer)
                }
            }
            FrameClass::VarFix => {
                if pointer <= 1 {
                    None
                } else {
                    Some(pointer - 1)
                }
            }
        };
        let mut result = Mapped {
            bands: Vec::with_capacity(count),
            attack,
            suppress_noise: Vec::with_capacity(count),
        };
        let mut last_lines = [false; 64];
        for (l, &fine) in syntax.high_resolution.iter().enumerate() {
            let frequency = if fine { &tables.high } else { &tables.low };
            let q = time
                .noise
                .windows(2)
                .position(|t| t[0] <= time.envelope[l] && time.envelope[l + 1] <= t[1])
                .ok_or_else(|| invalid("SBR envelope crosses a noise time boundary"))?;
            let mut lines = [false; 64];
            for (high_band, b) in tables.high.windows(2).enumerate() {
                let center = (usize::from(b[0]) + usize::from(b[1])) / 2;
                lines[center] =
                    harmonics[high_band] && (attack.is_none_or(|a| l >= a) || self.lines[center]);
            }
            let mut row = Vec::with_capacity(stop - kx);
            for band in kx..stop {
                let index = frequency
                    .windows(2)
                    .position(|b| usize::from(b[0]) <= band && band < usize::from(b[1]))
                    .unwrap();
                let q_index = tables
                    .noise
                    .windows(2)
                    .position(|b| usize::from(b[0]) <= band && band < usize::from(b[1]))
                    .unwrap();
                let left = usize::from(frequency[index]);
                let right = usize::from(frequency[index + 1]);
                row.push(Band {
                    target: envelope[l][index],
                    current: current[l][band - kx],
                    noise_ratio: noise[q][q_index],
                    harmonic_band: lines[left..right].iter().any(|x| *x),
                    harmonic_line: lines[band],
                });
            }
            result.bands.push(row);
            result
                .suppress_noise
                .push(attack == Some(l) || (l == 0 && self.carry_attack));
            last_lines = lines;
        }
        self.lines = last_lines;
        self.carry_attack = attack == Some(count);
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tables() -> FrequencyTables {
        FrequencyTables {
            master: vec![],
            high: vec![10, 11, 14, 18, 21, 28],
            low: vec![10, 11, 18, 28],
            noise: vec![10, 18, 28],
        }
    }
    fn syntax(class: FrameClass, pointer: u8) -> GridSyntax {
        GridSyntax {
            class,
            leading_offset: 0,
            trailing_offset: 0,
            leading_relative: if class == FrameClass::VarFix {
                vec![4, 4]
            } else {
                vec![]
            },
            trailing_relative: if class == FrameClass::FixVar {
                vec![4, 4]
            } else {
                vec![]
            },
            pointer,
            high_resolution: vec![false, true, false],
        }
    }
    fn parameters(grid: &GridSyntax) -> (Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let e = grid
            .high_resolution
            .iter()
            .enumerate()
            .map(|(l, fine)| {
                (0..if *fine { 5 } else { 3 })
                    .map(|b| ((l + 1) * 100 + b) as f64)
                    .collect()
            })
            .collect();
        (
            e,
            vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            vec![vec![5.0; 18]; 3],
        )
    }
    #[test]
    fn original_protocol_mapping_vectors_cover_attack_and_noise_splits() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-mapping-oracles.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 256);
        for case in cases {
            let class = match case["class"].as_str().unwrap() {
                "fixvar" => FrameClass::FixVar,
                _ => FrameClass::VarFix,
            };
            let grid = syntax(class, case["pointer"].as_u64().unwrap() as u8);
            let (e, q, current) = parameters(&grid);
            let harmonic: Vec<_> = case["harmonics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_bool().unwrap())
                .collect();
            let mut state = History::default();
            let mapped = state
                .map(&grid, 16, &tables(), &e, &q, &harmonic, &current)
                .unwrap();
            let expected_attack = case["attack"].as_u64().map(|x| x as usize);
            assert_eq!(mapped.attack, expected_attack);
            for (l, row) in mapped.bands.iter().enumerate() {
                assert_eq!(
                    mapped.suppress_noise[l],
                    case["suppress"][l].as_bool().unwrap()
                );
                for (band, x) in row.iter().enumerate() {
                    let expected = &case["expected"][l][band];
                    assert_eq!(x.target, expected[0].as_f64().unwrap());
                    assert_eq!(x.noise_ratio, expected[1].as_f64().unwrap());
                    assert_eq!(x.current, 5.0);
                    assert_eq!(x.harmonic_band, expected[2].as_bool().unwrap());
                    assert_eq!(x.harmonic_line, expected[3].as_bool().unwrap());
                }
                let levels =
                    super::super::aac_sbr_gain::calculate(row, mapped.suppress_noise[l]).unwrap();
                for (b, level) in row.iter().zip(levels) {
                    assert_eq!(level.sine > 0.0, b.harmonic_line);
                }
            }
        }
    }
    #[test]
    fn history_uses_absolute_lines_and_failed_frames_do_not_change_carry_or_lines() {
        let initial = syntax(FrameClass::VarFix, 0);
        let (e, q, c) = parameters(&initial);
        let harmonics = [true; 5];
        let mut state = History::default();
        state
            .map(&initial, 16, &tables(), &e, &q, &harmonics, &c)
            .unwrap();
        let deferred = syntax(FrameClass::FixVar, 1);
        let (e, q, c) = parameters(&deferred);
        let remembered = state
            .map(&deferred, 16, &tables(), &e, &q, &harmonics, &c)
            .unwrap();
        assert!(remembered.bands[0][0].harmonic_line);
        assert_eq!(remembered.attack, Some(3));
        let checkpoint = state.clone();
        let mut bad = c.clone();
        bad[2][17] = f64::NAN;
        assert!(
            state
                .map(&deferred, 16, &tables(), &e, &q, &harmonics, &bad)
                .is_err()
        );
        assert_eq!(state, checkpoint);
        let result = state
            .map(
                &initial,
                16,
                &tables(),
                &parameters(&initial).0,
                &q,
                &harmonics,
                &c,
            )
            .unwrap();
        assert!(result.suppress_noise[0]);
        assert!(!result.suppress_noise[1]);
        let mut replay = checkpoint.clone();
        let second = replay
            .map(
                &initial,
                16,
                &tables(),
                &parameters(&initial).0,
                &q,
                &harmonics,
                &c,
            )
            .unwrap();
        for (a, b) in result
            .bands
            .iter()
            .flatten()
            .zip(second.bands.iter().flatten())
        {
            assert_eq!(a.harmonic_line, b.harmonic_line);
        }
        state = checkpoint;
        let shifted = FrequencyTables {
            master: vec![],
            high: vec![9, 10, 14, 18, 21, 29],
            low: vec![9, 10, 18, 29],
            noise: vec![9, 18, 29],
        };
        let current = vec![vec![5.0; 20]; 3];
        let changed = state
            .map(&deferred, 16, &shifted, &e, &q, &harmonics, &current)
            .unwrap();
        assert!(!changed.bands[0][0].harmonic_line); // new center9 was not present at old center10
        assert!(!changed.bands[0][16].harmonic_line); // old center24, new center25
        state.reset();
        assert_eq!(state, History::default());
        let empty = state
            .map(
                &initial,
                16,
                &tables(),
                &parameters(&initial).0,
                &q,
                &[false; 5],
                &c,
            )
            .unwrap();
        assert!(empty.bands.iter().flatten().all(|b| !b.harmonic_band));
    }
    #[test]
    fn fixed_and_varvar_classes_cover_both_frame_sizes() {
        for slots in [15, 16] {
            for count in [1, 2, 4] {
                let grid = GridSyntax {
                    class: FrameClass::FixFix,
                    leading_offset: 0,
                    trailing_offset: 0,
                    leading_relative: vec![],
                    trailing_relative: vec![],
                    pointer: 0,
                    high_resolution: vec![true; count],
                };
                let time = grid.time_grid(slots).unwrap();
                let mut state = History::default();
                let mapped = state
                    .map(
                        &grid,
                        slots,
                        &tables(),
                        &vec![vec![64.0; 5]; count],
                        &vec![vec![1.0; 2]; time.noise.len() - 1],
                        &[true; 5],
                        &vec![vec![8.0; 18]; count],
                    )
                    .unwrap();
                assert_eq!(mapped.attack, None);
                assert_eq!(mapped.suppress_noise, vec![false; count]);
                assert_eq!(
                    mapped
                        .bands
                        .iter()
                        .flatten()
                        .filter(|b| b.harmonic_line)
                        .count(),
                    5 * count
                );
            }
            for pointer in 0..=3 {
                let grid = GridSyntax {
                    class: FrameClass::VarVar,
                    leading_offset: 1,
                    trailing_offset: 1,
                    leading_relative: vec![4],
                    trailing_relative: vec![4],
                    pointer,
                    high_resolution: vec![false, true, false],
                };
                let (e, q, c) = parameters(&grid);
                let mut state = History::default();
                let mapped = state
                    .map(&grid, slots, &tables(), &e, &q, &[true; 5], &c)
                    .unwrap();
                let attack = if pointer == 0 {
                    None
                } else {
                    Some(4 - usize::from(pointer))
                };
                assert_eq!(mapped.attack, attack);
                for l in 0..3 {
                    assert_eq!(mapped.suppress_noise[l], attack == Some(l));
                    assert_eq!(
                        mapped.bands[l].iter().filter(|b| b.harmonic_line).count(),
                        if attack.is_none_or(|a| l >= a) { 5 } else { 0 }
                    );
                }
            }
        }
    }
    #[test]
    fn bounded_harmonic_syntax_is_transactional_at_all_offsets_and_truncations() {
        for count in 1..=8 {
            for pattern in 0..1usize << count {
                for offset in 0..8 {
                    let mut bytes = vec![0u8; (offset + count + 7) / 8];
                    for bit in 0..count {
                        if pattern & (1 << (count - 1 - bit)) != 0 {
                            bytes[(offset + bit) / 8] |= 1 << (7 - (offset + bit) % 8);
                        }
                    }
                    for end in offset..offset + count {
                        let mut bits = BitReader::new(&bytes);
                        bits.read(offset as u8).unwrap();
                        assert!(read_harmonics(&mut bits, end, true, count).is_err());
                        assert_eq!(bits.position(), offset);
                    }
                    let mut bits = BitReader::new(&bytes);
                    bits.read(offset as u8).unwrap();
                    let values = read_harmonics(&mut bits, offset + count, true, count).unwrap();
                    assert_eq!(bits.position(), offset + count);
                    for (i, value) in values.iter().enumerate() {
                        assert_eq!(*value, pattern & (1 << (count - 1 - i)) != 0);
                    }
                    let start = bits.position();
                    assert_eq!(
                        read_harmonics(&mut bits, start, false, count).unwrap(),
                        vec![false; count]
                    );
                    assert_eq!(bits.position(), start);
                }
            }
        }
        for count in [0, 65] {
            assert!(read_harmonics(&mut BitReader::new(&[0; 9]), 72, true, count).is_err());
        }
    }
}
