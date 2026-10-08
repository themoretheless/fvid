//! Owned legacy SBR envelope/noise coefficient syntax and delta reconstruction.
use super::{
    Result,
    aac_sbr_controls::{DeltaDirection, DeltaFlags},
    aac_sbr_grid::GridSyntax,
    aac_sbr_huffman::Book,
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaRow {
    pub direction: DeltaDirection,
    /// Frequency rows begin with an unsigned absolute value; temporal rows are
    /// entirely signed Huffman deltas. Stereo balance scaling is not applied yet.
    pub values: Vec<i16>,
}
fn unsigned(bits: &mut BitReader<'_>, end: usize, width: u8) -> Result<i16> {
    if end < bits.position()
        || end - bits.position() > bits.remaining()
        || usize::from(width) > end - bits.position()
    {
        return Err(invalid("truncated SBR absolute coefficient"));
    }
    Ok(bits.read(width)? as i16)
}
fn row(
    bits: &mut BitReader<'_>,
    end: usize,
    count: usize,
    direction: DeltaDirection,
    width: u8,
    time: Book,
    frequency: Book,
) -> Result<DeltaRow> {
    let mut values = Vec::with_capacity(count);
    if direction == DeltaDirection::Frequency {
        values.push(unsigned(bits, end, width)?);
    }
    let book = if direction == DeltaDirection::Time {
        time
    } else {
        frequency
    };
    while values.len() < count {
        values.push(book.decode(bits, end)?);
    }
    Ok(DeltaRow { direction, values })
}

/// Read all channel envelopes atomically. `balance` identifies the second
/// channel of a coupled pair; uncoupled channels always use level books.
pub fn read_envelopes(
    bits: &mut BitReader<'_>,
    end: usize,
    grid: &GridSyntax,
    slots: u8,
    flags: &DeltaFlags,
    header_amplitude: bool,
    balance: bool,
    low_bands: usize,
    high_bands: usize,
) -> Result<Vec<DeltaRow>> {
    grid.time_grid(slots)?;
    if flags.envelope.len() != grid.high_resolution.len()
        || flags.noise.len() != grid.noise_envelopes()
        || !(1..=64).contains(&low_bands)
        || !(1..=64).contains(&high_bands)
    {
        return Err(invalid("invalid SBR envelope dimensions"));
    }
    let amplitude = grid.amplitude_resolution(header_amplitude);
    let (time, frequency, width) = match (balance, amplitude) {
        (false, false) => (Book::EnvelopeTime15, Book::EnvelopeFrequency15, 7),
        (false, true) => (Book::EnvelopeTime30, Book::EnvelopeFrequency30, 6),
        (true, false) => (Book::BalanceTime15, Book::BalanceFrequency15, 6),
        (true, true) => (Book::BalanceTime30, Book::BalanceFrequency30, 5),
    };
    let mut trial = bits.clone();
    let mut rows = Vec::with_capacity(flags.envelope.len());
    for (&high, &direction) in grid.high_resolution.iter().zip(&flags.envelope) {
        rows.push(row(
            &mut trial,
            end,
            if high { high_bands } else { low_bands },
            direction,
            width,
            time,
            frequency,
        )?);
    }
    *bits = trial;
    Ok(rows)
}

pub fn read_noise(
    bits: &mut BitReader<'_>,
    end: usize,
    grid: &GridSyntax,
    slots: u8,
    flags: &DeltaFlags,
    balance: bool,
    noise_bands: usize,
) -> Result<Vec<DeltaRow>> {
    grid.time_grid(slots)?;
    if flags.envelope.len() != grid.high_resolution.len()
        || flags.noise.len() != grid.noise_envelopes()
        || !(1..=5).contains(&noise_bands)
    {
        return Err(invalid("invalid SBR noise dimensions"));
    }
    let (time, frequency) = if balance {
        (Book::NoiseBalanceTime, Book::BalanceFrequency30)
    } else {
        (Book::NoiseTime, Book::EnvelopeFrequency30)
    };
    let mut trial = bits.clone();
    let mut rows = Vec::with_capacity(flags.noise.len());
    for &direction in &flags.noise {
        rows.push(row(
            &mut trial,
            end,
            noise_bands,
            direction,
            5,
            time,
            frequency,
        )?);
    }
    *bits = trial;
    Ok(rows)
}

/// Reconstruct quantized values, using physical frequency intervals to map a
/// previous envelope when the high/low frequency resolution changes. Previous
/// values are already scaled; balance doubles only newly transmitted data.
pub fn reconstruct(
    row: &DeltaRow,
    borders: &[u8],
    previous: Option<(&[u8], &[i16])>,
    balance: bool,
) -> Result<Vec<i16>> {
    fn valid(b: &[u8], n: usize) -> bool {
        n > 0
            && n <= 64
            && b.len() == n + 1
            && b.last().copied().unwrap_or(255) <= 64
            && b.windows(2).all(|w| w[0] < w[1])
    }
    if !valid(borders, row.values.len()) {
        return Err(invalid("invalid SBR coefficient borders"));
    }
    let old = if row.direction == DeltaDirection::Time {
        let (old_borders, values) =
            previous.ok_or_else(|| invalid("missing previous SBR envelope"))?;
        if !valid(old_borders, values.len())
            || old_borders[0] != borders[0]
            || old_borders.last() != borders.last()
            || !(borders.iter().all(|x| old_borders.binary_search(x).is_ok())
                || old_borders.iter().all(|x| borders.binary_search(x).is_ok()))
        {
            return Err(invalid("incompatible previous SBR envelope"));
        }
        Some((old_borders, values))
    } else {
        None
    };
    let mut result = Vec::with_capacity(row.values.len());
    for (i, &delta) in row.values.iter().enumerate() {
        let delta = delta
            .checked_mul(if balance { 2 } else { 1 })
            .ok_or_else(|| invalid("overflowing SBR balance delta"))?;
        let base = if let Some((b, v)) = old {
            let index = b.partition_point(|x| *x <= borders[i]) - 1;
            v[index]
        } else if i == 0 {
            0
        } else {
            result[i - 1]
        };
        result.push(
            base.checked_add(delta)
                .ok_or_else(|| invalid("overflowing SBR reconstructed coefficient"))?,
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::aac_sbr_grid::FrameClass;
    use super::*;
    fn grid(single: bool) -> GridSyntax {
        GridSyntax {
            class: if single {
                FrameClass::FixFix
            } else {
                FrameClass::VarFix
            },
            leading_offset: 0,
            trailing_offset: 0,
            leading_relative: if single { vec![] } else { vec![2] },
            trailing_relative: vec![],
            pointer: 0,
            high_resolution: if single {
                vec![true]
            } else {
                vec![true, false]
            },
        }
    }
    fn packed(s: &str, start: usize) -> Vec<u8> {
        let text = "1".repeat(start) + s + "1111111111111111";
        let mut b = vec![0; text.len().div_ceil(8)];
        for (i, v) in text.bytes().enumerate() {
            if v == b'1' {
                b[i / 8] |= 1 << (7 - i % 8);
            }
        }
        b
    }
    fn code(book: usize, delta: i16) -> String {
        let tables: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-huffman-codewords.json"
        ))
        .unwrap();
        let index =
            (delta + i16::try_from(tables[book]["offset"].as_i64().unwrap()).unwrap()) as usize;
        let r = &tables[book]["rows"][index];
        format!(
            "{:0width$b}",
            r[2].as_u64().unwrap(),
            width = r[1].as_u64().unwrap() as usize
        )
    }
    #[test]
    fn level_balance_and_both_resolutions_read_mixed_direction_rows_atomically() {
        for (balance, amplitude, tbook, fbook, width) in [
            (false, false, 0, 1, 7),
            (true, false, 2, 3, 6),
            (false, true, 4, 5, 6),
            (true, true, 6, 7, 5),
        ] {
            let s = format!(
                "{:0width$b}{}{}{}{}",
                10,
                code(fbook, 1),
                code(fbook, -2),
                code(tbook, 2),
                code(tbook, -1),
                width = width
            );
            let flags = DeltaFlags {
                envelope: vec![DeltaDirection::Frequency, DeltaDirection::Time],
                noise: vec![DeltaDirection::Frequency; 2],
            };
            for start in 0..8 {
                let bytes = packed(&s, start);
                let mut bits = BitReader::new(&bytes);
                bits.skip(start).unwrap();
                for end in start..start + s.len() {
                    assert!(
                        read_envelopes(
                            &mut bits,
                            end,
                            &grid(false),
                            16,
                            &flags,
                            amplitude,
                            balance,
                            2,
                            3
                        )
                        .is_err()
                    );
                    assert_eq!(bits.position(), start);
                }
                let r = read_envelopes(
                    &mut bits,
                    start + s.len(),
                    &grid(false),
                    16,
                    &flags,
                    amplitude,
                    balance,
                    2,
                    3,
                )
                .unwrap();
                assert_eq!(r[0].values, [10, 1, -2]);
                assert_eq!(r[1].values, [2, -1]);
                assert_eq!(bits.position(), start + s.len());
                assert_eq!(bits.read(8).unwrap(), 255);
            }
        }
    }
    #[test]
    fn noise_uses_dedicated_time_books_but_shared_frequency_books() {
        for (balance, tbook, fbook) in [(false, 8, 5), (true, 9, 7)] {
            let s = format!(
                "01010{}{}{}{}{}",
                code(fbook, 1),
                code(fbook, -2),
                code(tbook, 2),
                code(tbook, -1),
                code(tbook, 3)
            );
            let bytes = packed(&s, 0);
            let mut bits = BitReader::new(&bytes);
            let flags = DeltaFlags {
                envelope: vec![DeltaDirection::Frequency; 2],
                noise: vec![DeltaDirection::Frequency, DeltaDirection::Time],
            };
            for end in 0..s.len() {
                assert!(read_noise(&mut bits, end, &grid(false), 15, &flags, balance, 3).is_err());
                assert_eq!(bits.position(), 0);
            }
            let r = read_noise(&mut bits, s.len(), &grid(false), 15, &flags, balance, 3).unwrap();
            assert_eq!(r[0].values, [10, 1, -2]);
            assert_eq!(r[1].values, [2, -1, 3]);
            assert_eq!(bits.position(), s.len());
        }
    }
    #[test]
    fn independent_delta_results_cover_resolution_changes_and_balance_scale() {
        let frequency = DeltaRow {
            direction: DeltaDirection::Frequency,
            values: vec![10, 1, -2],
        };
        assert_eq!(
            reconstruct(&frequency, &[10, 12, 16, 20], None, false).unwrap(),
            [10, 11, 9]
        );
        assert_eq!(
            reconstruct(&frequency, &[10, 12, 16, 20], None, true).unwrap(),
            [20, 22, 18]
        );
        let time = DeltaRow {
            direction: DeltaDirection::Time,
            values: vec![2, -1, 3],
        };
        assert_eq!(
            reconstruct(
                &time,
                &[10, 12, 16, 20],
                Some((&[10, 16, 20], &[20, 40])),
                false
            )
            .unwrap(),
            [22, 19, 43]
        );
        assert_eq!(
            reconstruct(
                &time,
                &[10, 12, 16, 20],
                Some((&[10, 16, 20], &[20, 40])),
                true
            )
            .unwrap(),
            [24, 18, 46]
        );
        let low = DeltaRow {
            direction: DeltaDirection::Time,
            values: vec![1, -2],
        };
        assert_eq!(
            reconstruct(
                &low,
                &[10, 16, 20],
                Some((&[10, 12, 16, 20], &[10, 11, 9])),
                false
            )
            .unwrap(),
            [11, 7]
        );
        assert!(reconstruct(&time, &[10, 12, 16, 20], None, false).is_err());
        let overflow = DeltaRow {
            direction: DeltaDirection::Frequency,
            values: vec![i16::MAX, 1],
        };
        assert!(reconstruct(&overflow, &[10, 12, 20], None, false).is_err());
        assert!(reconstruct(&overflow, &[10, 12, 20], None, true).is_err());
    }
    #[test]
    fn single_fixed_envelope_overrides_header_and_invalid_dimensions_are_atomic() {
        let flags = DeltaFlags {
            envelope: vec![DeltaDirection::Frequency],
            noise: vec![DeltaDirection::Frequency],
        };
        let s = "1010101"; // Seven-bit absolute level = 85 despite coarse header resolution.
        let bytes = packed(s, 0);
        let mut bits = BitReader::new(&bytes);
        let r = read_envelopes(&mut bits, 7, &grid(true), 16, &flags, true, false, 1, 1).unwrap();
        assert_eq!(r[0].values, [85]);
        assert_eq!(bits.position(), 7);
        let mut bits = BitReader::new(&bytes);
        for count in [0, 65, usize::MAX] {
            assert!(
                read_envelopes(
                    &mut bits,
                    7,
                    &grid(true),
                    16,
                    &flags,
                    false,
                    false,
                    count,
                    1
                )
                .is_err()
            );
            assert_eq!(bits.position(), 0);
        }
        for count in [0, 6, usize::MAX] {
            assert!(read_noise(&mut bits, 7, &grid(true), 16, &flags, false, count).is_err());
            assert_eq!(bits.position(), 0);
        }
        let bad = DeltaFlags {
            envelope: vec![],
            noise: vec![],
        };
        assert!(read_envelopes(&mut bits, 7, &grid(true), 16, &bad, false, false, 1, 1).is_err());
        assert_eq!(bits.position(), 0);
        let time = DeltaRow {
            direction: DeltaDirection::Time,
            values: vec![0, 0],
        };
        assert!(reconstruct(&time, &[10, 14, 20], Some((&[10, 12, 20], &[1, 2])), false).is_err());
    }
}
