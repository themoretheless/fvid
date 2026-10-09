//! Owned SBR quantized coefficient history and frame dequantization.
use super::{
    aac_sbr_coefficients::reconstruct, aac_sbr_controls::DeltaDirection, aac_sbr_data::Data,
    aac_sbr_dequant, aac_sbr_extension, bits::BitReader, invalid, Result,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Previous {
    envelope: Option<(Vec<u8>, Vec<i16>)>,
    noise: Option<Vec<i16>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    channels: Vec<Previous>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub quantized_envelope: Vec<Vec<i16>>,
    pub quantized_noise: Vec<Vec<i16>>,
    pub envelope: Vec<Vec<f64>>,
    pub noise: Vec<Vec<f64>>,
    pub coarse: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    pub channels: Vec<Channel>,
}

fn borders(values: &[u8]) -> bool {
    (2..=65).contains(&values.len())
        && values[0] > 0
        && values[values.len() - 1] <= 64
        && values.windows(2).all(|v| v[0] < v[1])
}
impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Recover rows within and across frames, then dequantize both channels.
    /// Previous envelopes retain their physical borders for high/low mapping;
    /// noise references use the previous row's band *index* (4.6.18.3.4).
    /// Quantized g_E values are retained as transmitted/reconstructed; current
    /// amplitude resolution applies at the subsequent dequantization stage.
    /// Seek/format reset is explicit. Frequency-coded rows establish fresh
    /// references; temporal rows require a compatible previous reference.
    /// No state is committed on any reconstruction or dequantization failure.
    pub fn decode(&mut self, data: &Data, header_amplitude: bool, slots: u8) -> Result<Parameters> {
        let tables = &data.frequency;
        let count = data.channels.len();
        if !(1..=2).contains(&count)
            || (data.coupled && count != 2)
            || !borders(&tables.master)
            || !borders(&tables.high)
            || !borders(&tables.low)
            || !borders(&tables.noise)
            || tables.high[0] > 32
            || tables.noise.len() > 6
            || tables.high.last() != tables.low.last()
            || tables.high.last() != tables.noise.last()
            || tables.high[0] != tables.low[0]
            || tables.high[0] != tables.noise[0]
            || !tables.high.iter().all(|v| tables.master.contains(v))
            || !tables.low.iter().all(|v| tables.high.contains(v))
            || !tables.noise.iter().all(|v| tables.low.contains(v))
        {
            return Err(invalid("invalid SBR frame coefficient geometry"));
        }
        if !self.channels.is_empty() && self.channels.len() != count {
            return Err(invalid("SBR channel history requires stream reset"));
        }
        if data.coupled
            && (data.channels[0].grid != data.channels[1].grid
                || data.channels[0].inverse_filter != data.channels[1].inverse_filter)
        {
            return Err(invalid("inconsistent coupled SBR channel controls"));
        }
        let mut trial = self.clone();
        if trial.channels.is_empty() {
            trial.channels.resize_with(count, Previous::default);
        }
        let mut result = Vec::with_capacity(count);
        for (index, syntax) in data.channels.iter().enumerate() {
            let grid = syntax.grid.time_grid(slots)?;
            if syntax.envelope.len() != grid.envelope.len() - 1
                || syntax.noise.len() != grid.noise.len() - 1
                || syntax.delta.envelope.len() != syntax.envelope.len()
                || syntax.delta.noise.len() != syntax.noise.len()
                || syntax.inverse_filter.len() + 1 != tables.noise.len()
                || syntax.harmonics.len() + 1 != tables.high.len()
                || syntax
                    .envelope
                    .iter()
                    .zip(&syntax.delta.envelope)
                    .any(|(r, d)| r.direction != *d)
                || syntax
                    .noise
                    .iter()
                    .zip(&syntax.delta.noise)
                    .any(|(r, d)| r.direction != *d)
            {
                return Err(invalid("invalid SBR frame coefficient dimensions"));
            }
            let balance = data.coupled && index == 1;
            let previous = &mut trial.channels[index];
            let mut envelope = Vec::with_capacity(syntax.envelope.len());
            for (row, high) in syntax.envelope.iter().zip(&syntax.grid.high_resolution) {
                let current = if *high { &tables.high } else { &tables.low };
                let old = previous
                    .envelope
                    .as_ref()
                    .map(|(b, v)| (b.as_slice(), v.as_slice()));
                let values = reconstruct(row, current, old, balance)?;
                previous.envelope = Some((current.clone(), values.clone()));
                envelope.push(values);
            }
            let mut noise = Vec::with_capacity(syntax.noise.len());
            for row in &syntax.noise {
                let old = if row.direction == DeltaDirection::Time {
                    let values = previous
                        .noise
                        .as_ref()
                        .filter(|v| v.len() >= row.values.len())
                        .ok_or_else(|| invalid("missing previous SBR noise bands"))?;
                    Some((tables.noise.as_slice(), &values[..row.values.len()]))
                } else {
                    None
                };
                let values = reconstruct(row, &tables.noise, old, balance)?;
                previous.noise = Some(values.clone());
                noise.push(values);
            }
            result.push(Channel {
                quantized_envelope: envelope,
                quantized_noise: noise,
                envelope: vec![],
                noise: vec![],
                coarse: syntax.grid.amplitude_resolution(header_amplitude),
            });
        }
        if data.coupled {
            let (left, right) = result.split_at_mut(1);
            let (left, right) = (&mut left[0], &mut right[0]);
            for (levels, balances) in left
                .quantized_envelope
                .iter()
                .zip(&right.quantized_envelope)
            {
                let mut l = Vec::with_capacity(levels.len());
                let mut r = Vec::with_capacity(levels.len());
                for (&level, &balance) in levels.iter().zip(balances) {
                    let value = aac_sbr_dequant::coupled_envelope(level, balance, left.coarse)?;
                    l.push(value.left);
                    r.push(value.right);
                }
                left.envelope.push(l);
                right.envelope.push(r);
            }
            for (levels, balances) in left.quantized_noise.iter().zip(&right.quantized_noise) {
                let mut l = Vec::with_capacity(levels.len());
                let mut r = Vec::with_capacity(levels.len());
                for (&level, &balance) in levels.iter().zip(balances) {
                    let value = aac_sbr_dequant::coupled_noise(level, balance)?;
                    l.push(value.left);
                    r.push(value.right);
                }
                left.noise.push(l);
                right.noise.push(r);
            }
        } else {
            for channel in &mut result {
                channel.envelope = channel
                    .quantized_envelope
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|&v| aac_sbr_dequant::envelope(v, channel.coarse))
                            .collect::<Result<Vec<_>>>()
                    })
                    .collect::<Result<Vec<_>>>()?;
                channel.noise = channel
                    .quantized_noise
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|&v| aac_sbr_dequant::noise(v))
                            .collect::<Result<Vec<_>>>()
                    })
                    .collect::<Result<Vec<_>>>()?;
            }
        }
        *self = trial;
        Ok(Parameters { channels: result })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    syntax: aac_sbr_extension::State,
    coefficients: History,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub syntax: aac_sbr_extension::Frame,
    pub parameters: Parameters,
}
impl Frame {
    /// Retained vector payload of this parsed/dequantized frame; excludes stack fields.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut f = super::memory::Footprint::new();
        let count = (|| -> std::result::Result<(), String> {
            let d = &self.syntax.data;
            for v in [
                &d.frequency.master,
                &d.frequency.high,
                &d.frequency.low,
                &d.frequency.noise,
            ] {
                f.vector(v)?;
            }
            f.vector(&d.channels)?;
            if let Some(v) = &d.extended_data {
                f.vector(v)?;
            }
            for c in &d.channels {
                f.vector(&c.grid.leading_relative)?;
                f.vector(&c.grid.trailing_relative)?;
                f.vector(&c.grid.high_resolution)?;
                f.vector(&c.delta.envelope)?;
                f.vector(&c.delta.noise)?;
                f.vector(&c.inverse_filter)?;
                f.vector(&c.harmonics)?;
                for rows in [&c.envelope, &c.noise] {
                    f.vector(rows)?;
                    for row in rows {
                        f.vector(&row.values)?;
                    }
                }
            }
            f.vector(&self.parameters.channels)?;
            for c in &self.parameters.channels {
                for rows in [&c.quantized_envelope, &c.quantized_noise] {
                    f.vector(rows)?;
                    for row in rows {
                        f.vector(row)?;
                    }
                }
                for rows in [&c.envelope, &c.noise] {
                    f.vector(rows)?;
                    for row in rows {
                        f.vector(row)?;
                    }
                }
            }
            Ok(())
        })();
        count.map_err(|e| invalid(&e))?;
        Ok(f.total())
    }
}
impl Stream {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        footprint.vector(&self.coefficients.channels)?;
        for previous in &self.coefficients.channels {
            if let Some((borders, values)) = &previous.envelope {
                footprint.vector(borders)?;
                footprint.vector(values)?;
            }
            if let Some(values) = &previous.noise {
                footprint.vector(values)?;
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Compose extension/CRC parsing and temporal reconstruction. Only a fully
    /// dequantized frame updates the input, saved header and coefficient state.
    /// This does not perform PCM synthesis or interpret PS extension bytes.
    pub fn read(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        crc: bool,
        sbr_rate: u32,
        slots: u8,
        channels: usize,
    ) -> Result<Frame> {
        let mut state = self.clone();
        let mut trial = bits.clone();
        let syntax = state
            .syntax
            .read(&mut trial, end, crc, sbr_rate, slots, channels)?;
        if syntax.format_reset {
            state.coefficients.reset();
        }
        let parameters =
            state
                .coefficients
                .decode(&syntax.data, syntax.header.amplitude_resolution, slots)?;
        *self = state;
        *bits = trial;
        Ok(Frame { syntax, parameters })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    const BINARY: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-history-syntax.bin");
    fn manifest() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-history-decimal.json"
        ))
        .unwrap()
    }
    fn raw(frame: &Value) -> &[u8] {
        &BINARY[frame["offset"].as_u64().unwrap() as usize..]
            [..frame["byte_length"].as_u64().unwrap() as usize]
    }
    fn read(stream: &mut Stream, frame: &Value, slots: u8, count: usize) -> Result<Frame> {
        let bytes = raw(frame);
        let mut bits = BitReader::new(bytes);
        let extension = bits.read(4)?;
        let frame = stream.read(
            &mut bits,
            bytes.len() * 8,
            extension == 14,
            48_000,
            slots,
            count,
        )?;
        assert_eq!(bits.remaining(), 0);
        Ok(frame)
    }
    fn compare_rows(actual: &[Vec<f64>], expected: &Value) {
        let expected = expected.as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (row, expected) in actual.iter().zip(expected) {
            let expected = expected.as_array().unwrap();
            assert_eq!(row.len(), expected.len());
            for (value, expected) in row.iter().zip(expected) {
                let expected: f64 = expected.as_str().unwrap().parse().unwrap();
                assert!(
                    (value - expected).abs() <= expected.abs() * 3e-14,
                    "{value} != {expected}"
                );
            }
        }
    }
    fn compare(actual: &Parameters, expected: &Value) {
        let expected = expected.as_array().unwrap();
        assert_eq!(actual.channels.len(), expected.len());
        for (channel, expected) in actual.channels.iter().zip(expected) {
            assert_eq!(
                json!(channel.quantized_envelope),
                expected["quantized_envelope"]
            );
            assert_eq!(json!(channel.quantized_noise), expected["quantized_noise"]);
            assert_eq!(channel.coarse, expected["coarse"].as_bool().unwrap());
            compare_rows(&channel.envelope, &expected["envelope"]);
            compare_rows(&channel.noise, &expected["noise"]);
        }
    }

    #[test]
    fn parsed_multiframe_parameters_match_dense_bin_and_decimal_oracles() {
        let manifest = manifest();
        assert_eq!(manifest["sequences"].as_array().unwrap().len(), 12);
        for sequence in manifest["sequences"].as_array().unwrap() {
            let slots = sequence["slots"].as_u64().unwrap() as u8;
            let count = if sequence["kind"] == "mono" { 1 } else { 2 };
            let mut stream = Stream::default();
            for (index, frame) in sequence["frames"].as_array().unwrap().iter().enumerate() {
                let checkpoint = stream.clone();
                let result = read(&mut stream, frame, slots, count).unwrap();
                compare(&result.parameters, &frame["channels"]);
                assert_eq!(result.syntax.header_reset, index == 0);
                assert_eq!(result.syntax.format_reset, index == 0);
                let mut replay = checkpoint;
                assert_eq!(read(&mut replay, frame, slots, count).unwrap(), result);
                assert_eq!(replay, stream);
            }
            // Seeking returns to the same initial state/result, not merely the
            // same row sizes. Subsequent temporal values must replay exactly.
            stream.reset();
            assert_eq!(stream, Stream::default());
            for frame in sequence["frames"].as_array().unwrap() {
                compare(
                    &read(&mut stream, frame, slots, count).unwrap().parameters,
                    &frame["channels"],
                );
            }
        }
    }

    #[test]
    fn late_dequantization_failure_restores_reader_header_and_both_channels() {
        let manifest = manifest();
        for sequence in manifest["sequences"].as_array().unwrap() {
            let slots = sequence["slots"].as_u64().unwrap() as u8;
            let count = if sequence["kind"] == "mono" { 1 } else { 2 };
            let mut stream = Stream::default();
            for frame in sequence["frames"].as_array().unwrap().iter().take(2) {
                read(&mut stream, frame, slots, count).unwrap();
            }
            let checkpoint = stream.clone();
            let bytes = raw(&sequence["invalid_noise"]);
            // Prove this vector reaches the intended noise-range error:
            // its CRC/header/data syntax is accepted independently first.
            let mut syntax = stream.syntax.clone();
            let mut reader = BitReader::new(bytes);
            assert_eq!(reader.read(4).unwrap(), 13);
            let frame = syntax
                .read(&mut reader, bytes.len() * 8, false, 48_000, slots, count)
                .unwrap();
            assert!(!frame.header.amplitude_resolution); // pending header change
            assert_ne!(syntax, stream.syntax);
            let error = stream
                .coefficients
                .clone()
                .decode(&frame.data, false, slots)
                .unwrap_err();
            assert!(
                error.to_string().contains("noise")
                    && error.to_string().contains("normative range"),
                "{error}"
            );
            let mut reader = BitReader::new(bytes);
            reader.skip(4).unwrap();
            let error = stream
                .read(&mut reader, bytes.len() * 8, false, 48_000, slots, count)
                .unwrap_err();
            assert!(error.to_string().contains("normative range"), "{error}");
            assert_eq!(reader.position(), 4);
            assert_eq!(stream, checkpoint);
            // The following headerless frame still sees the original coarse
            // header and last coefficients after the failed update.
            let frame = &sequence["frames"][2];
            compare(
                &read(&mut stream, frame, slots, count).unwrap().parameters,
                &frame["channels"],
            );
        }
    }

    #[test]
    fn truncated_stream_reads_and_missing_history_are_transactional() {
        let manifest = manifest();
        for sequence in manifest["sequences"].as_array().unwrap() {
            let slots = sequence["slots"].as_u64().unwrap() as u8;
            let count = if sequence["kind"] == "mono" { 1 } else { 2 };
            let mut stream = Stream::default();
            read(&mut stream, &sequence["frames"][0], slots, count).unwrap();
            let checkpoint = stream.clone();
            let bytes = raw(&sequence["frames"][1]);
            let mut reader = BitReader::new(bytes);
            reader.skip(4).unwrap();
            for end in 4..bytes.len() * 8 {
                assert!(stream
                    .read(&mut reader, end, true, 48_000, slots, count)
                    .is_err());
                assert_eq!(reader.position(), 4);
                assert_eq!(stream, checkpoint);
            }
            let mut empty = Stream::default();
            assert!(read(&mut empty, &sequence["frames"][1], slots, count)
                .unwrap_err()
                .to_string()
                .contains("missing previous SBR envelope"));
            assert_eq!(empty, Stream::default());
            let mut data = read(&mut stream, &sequence["frames"][1], slots, count)
                .unwrap()
                .syntax
                .data;
            data.channels[0].noise.pop();
            let checkpoint = stream.coefficients.clone();
            assert!(stream.coefficients.decode(&data, true, slots).is_err());
            assert_eq!(stream.coefficients, checkpoint);
            data.frequency.high.clear();
            assert!(stream.coefficients.decode(&data, true, slots).is_err());
            assert_eq!(stream.coefficients, checkpoint);
        }
    }

    #[test]
    fn noise_history_uses_band_indices_when_frequency_borders_change() {
        use super::super::{
            aac_sbr_bands::FrequencyTables,
            aac_sbr_coefficients::DeltaRow,
            aac_sbr_controls::{DeltaFlags, InverseFilterMode},
            aac_sbr_data::Channel as SyntaxChannel,
            aac_sbr_grid::{FrameClass, GridSyntax},
        };
        let old = FrequencyTables::from_qmf_bounds(10, 30, 0, false, 0, 3).unwrap();
        let new = FrequencyTables::from_qmf_bounds(10, 30, 3, true, 0, 3).unwrap();
        assert_eq!(old.noise.len(), 6);
        assert_eq!(new.noise.len(), 6);
        assert_ne!(old.noise, new.noise);
        let frame = |frequency: FrequencyTables, direction, noise_values| {
            let mut envelope_values = vec![0; frequency.high.len() - 1];
            envelope_values[0] = 20;
            let channel = SyntaxChannel {
                grid: GridSyntax {
                    class: FrameClass::FixFix,
                    leading_offset: 0,
                    trailing_offset: 0,
                    leading_relative: vec![],
                    trailing_relative: vec![],
                    pointer: 0,
                    high_resolution: vec![true],
                },
                delta: DeltaFlags {
                    envelope: vec![DeltaDirection::Frequency],
                    noise: vec![direction],
                },
                inverse_filter: vec![InverseFilterMode::Off; 5],
                envelope: vec![DeltaRow {
                    direction: DeltaDirection::Frequency,
                    values: envelope_values,
                }],
                noise: vec![DeltaRow {
                    direction,
                    values: noise_values,
                }],
                harmonics: vec![false; frequency.high.len() - 1],
            };
            Data {
                frequency,
                coupled: false,
                channels: vec![channel],
                extended_data: None,
            }
        };
        let mut history = History::default();
        let initial = history
            .decode(
                &frame(old, DeltaDirection::Frequency, vec![12, 1, 2, 3, 4]),
                false,
                16,
            )
            .unwrap();
        assert_eq!(
            initial.channels[0].quantized_noise,
            [vec![12, 13, 15, 18, 22]]
        );
        let next = history
            .decode(
                &frame(new, DeltaDirection::Time, vec![-1, 0, 1, 0, -1]),
                false,
                16,
            )
            .unwrap();
        // Q(k,l) references Q'(k,last), not the previous interval containing k.
        assert_eq!(next.channels[0].quantized_noise, [vec![11, 13, 16, 18, 21]]);
    }
}
