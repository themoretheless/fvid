//! Transactional composition of owned SBR analysis, HF generation and mapping.
//! This prepares envelope adjustment; it does not yet synthesize PCM or PS.
use super::{
    Result,
    aac_sbr_bands::FrequencyTables,
    aac_sbr_buffers::LowDelay,
    aac_sbr_chirp::Chirp,
    aac_sbr_energy, aac_sbr_hf,
    aac_sbr_history::Frame,
    aac_sbr_mapping::{History, Mapped},
    aac_sbr_qmf::{Analysis, Complex},
    invalid,
};
#[derive(Clone, Debug, PartialEq)]
struct ChannelState {
    analysis: Analysis,
    delay: LowDelay,
    chirp: Chirp,
    mapping: History,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preparation {
    channels: Vec<ChannelState>,
    format: Option<(u32, u8, usize)>,
    frequency: Option<FrequencyTables>,
}
pub struct Channel {
    pub low: Vec<[Complex; 32]>,
    pub high: Vec<[Complex; 64]>,
    pub energy: Vec<Vec<f64>>,
    pub mapped: Mapped,
}
impl Preparation {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// PCM is normalized core AAC output, exactly 960/1024 samples per channel.
    /// Analysis columns are converted to the standard's 16-bit QMF units before
    /// prediction/energy estimation (gain epsilon=1 uses those same units).
    /// A header geometry change resets frequency-dependent chirp state while
    /// preserving delayed low columns and absolute harmonic/attack history.
    /// Format changes start a new pipeline. Both channels commit together.
    pub fn process(
        &mut self,
        frame: &Frame,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
    ) -> Result<Vec<Channel>> {
        let data = &frame.syntax.data;
        let header = &frame.syntax.header;
        if !matches!(slots, 15 | 16)
            || !matches!(pcm.len(), 1 | 2)
            || data.channels.len() != pcm.len()
            || frame.parameters.channels.len() != pcm.len()
            || data.coupled && pcm.len() != 2
            || pcm
                .iter()
                .any(|p| p.len() != 64 * usize::from(slots) || p.iter().any(|x| !x.is_finite()))
            || FrequencyTables::from_header(header, rate)? != data.frequency
        {
            return Err(invalid("invalid SBR preparation frame"));
        }
        let format = (rate, slots, pcm.len());
        let mut trial = self.clone();
        if trial.format.is_some_and(|old| old != format) && !frame.syntax.format_reset {
            return Err(invalid("SBR preparation format changed without reset"));
        }
        if frame.syntax.format_reset {
            trial.reset();
        }
        if trial
            .frequency
            .as_ref()
            .is_some_and(|old| old != &data.frequency)
            && !frame.syntax.header_reset
        {
            return Err(invalid("SBR preparation frequency changed without reset"));
        }
        let noise_count = data.frequency.noise.len() - 1;
        if trial.channels.is_empty() {
            for _ in pcm {
                trial.channels.push(ChannelState {
                    analysis: Analysis::default(),
                    delay: LowDelay::default(),
                    chirp: Chirp::new(noise_count)?,
                    mapping: History::default(),
                });
            }
        } else if frame.syntax.header_reset {
            for channel in &mut trial.channels {
                channel.chirp = Chirp::new(noise_count)?;
            }
        }
        let kx = data.frequency.high[0];
        let patches = aac_sbr_hf::patches(&data.frequency.master, kx, rate)?;
        let mut output = Vec::with_capacity(pcm.len());
        for (index, &samples) in pcm.iter().enumerate() {
            let state = &mut trial.channels[index];
            let syntax = &data.channels[index];
            let values = &frame.parameters.channels[index];
            let grid = syntax.grid.time_grid(slots)?;
            let mut analysis = state.analysis.process(samples)?;
            for row in &mut analysis {
                for value in row {
                    value.re *= 32768.0;
                    value.im *= 32768.0;
                }
            }
            let low = state.delay.process(&analysis, slots, kx)?;
            let bandwidth = state.chirp.update(&syntax.inverse_filter)?;
            let high = aac_sbr_hf::generate(
                &low,
                slots,
                &grid,
                data.frequency.master[0],
                &patches,
                &data.frequency.noise,
                bandwidth,
            )?;
            let energy = aac_sbr_energy::estimate(
                &high,
                slots,
                &grid,
                &syntax.grid.high_resolution,
                &data.frequency,
                header.interpolate_frequency,
            )?;
            let mapped = state.mapping.map(
                &syntax.grid,
                slots,
                &data.frequency,
                &values.envelope,
                &values.noise,
                &syntax.harmonics,
                &energy,
            )?;
            output.push(Channel {
                low,
                high,
                energy,
                mapped,
            });
        }
        trial.format = Some(format);
        trial.frequency = Some(data.frequency.clone());
        *self = trial;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::super::{aac_sbr_history::Stream, bits::BitReader};
    use super::*;
    use serde_json::Value;
    fn fixtures() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-history-decimal.json"
        ))
        .unwrap()
    }
    fn read(stream: &mut Stream, value: &Value, slots: u8, count: usize) -> Frame {
        let binary =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-history-syntax.bin");
        let offset = value["offset"].as_u64().unwrap() as usize;
        let n = value["byte_length"].as_u64().unwrap() as usize;
        let mut bits = BitReader::new(&binary[offset..offset + n]);
        let kind = bits.read(4).unwrap();
        stream
            .read(&mut bits, n * 8, kind == 14, 48000, slots, count)
            .unwrap()
    }
    fn equal(a: &[Channel], b: &[Channel]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(a.low, b.low);
            assert_eq!(a.high, b.high);
            assert_eq!(a.energy, b.energy);
            assert_eq!(a.mapped.suppress_noise, b.mapped.suppress_noise);
            for (a, b) in a
                .mapped
                .bands
                .iter()
                .flatten()
                .zip(b.mapped.bands.iter().flatten())
            {
                assert_eq!(
                    (
                        a.target,
                        a.current,
                        a.noise_ratio,
                        a.harmonic_band,
                        a.harmonic_line
                    ),
                    (
                        b.target,
                        b.current,
                        b.noise_ratio,
                        b.harmonic_band,
                        b.harmonic_line
                    )
                );
            }
        }
    }
    #[test]
    fn normalized_pcm_reaches_hf_in_standard_units_against_direct_convolution_oracles() {
        let fixture = fixtures();
        let case = &fixture["sequences"][0];
        let slots = case["slots"].as_u64().unwrap() as u8;
        let frame = read(&mut Stream::default(), &case["frames"][0], slots, 1);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let kx = usize::from(frame.syntax.data.frequency.high[0]);
        for name in ["impulse-first", "impulse-boundary", "dense", "tones"] {
            let raw = std::fs::read(root.join(format!("aac-sbr-qmf-{name}.f32le"))).unwrap();
            let expected =
                std::fs::read(root.join(format!("aac-sbr-qmf-{name}.complex-f64le"))).unwrap();
            let mut pcm: Vec<_> = raw
                .chunks_exact(4)
                .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
                .collect();
            pcm.resize(64 * usize::from(slots), 0.0);
            let output = Preparation::default()
                .process(&frame, &[&pcm], 48000, slots)
                .unwrap();
            assert!(
                output[0].low[..8]
                    .iter()
                    .flatten()
                    .all(|x| *x == Complex::default())
            );
            for (index, bytes) in expected.chunks_exact(16).enumerate() {
                let k = index % 32;
                let l = index / 32;
                let actual = output[0].low[l + 8][k];
                if k >= kx {
                    assert_eq!(actual, Complex::default());
                    continue;
                }
                let re = f64::from_le_bytes(bytes[..8].try_into().unwrap()) * 32768.0;
                let im = f64::from_le_bytes(bytes[8..].try_into().unwrap()) * 32768.0;
                assert!(
                    (actual.re - re).abs() < 32768.0 * 2e-12
                        && (actual.im - im).abs() < 32768.0 * 2e-12,
                    "{name} l={l} k={k}"
                );
            }
        }
    }
    #[test]
    fn parsed_four_frame_sequences_compose_analysis_hf_energy_mapping_and_replay() {
        let fixture = fixtures();
        for case in fixture["sequences"].as_array().unwrap() {
            let slots = case["slots"].as_u64().unwrap() as u8;
            let count = if case["kind"] == "mono" { 1 } else { 2 };
            let mut stream = Stream::default();
            let mut engine = Preparation::default();
            for (index, value) in case["frames"].as_array().unwrap().iter().enumerate() {
                let frame = read(&mut stream, value, slots, count);
                let pcm: Vec<Vec<f32>> = (0..count)
                    .map(|c| {
                        (0..64 * usize::from(slots))
                            .map(|i| {
                                if index == 0 {
                                    0.0
                                } else {
                                    (((i * 13 + c * 7 + index * 3) % 31) as f32 - 15.0) / 64.0
                                }
                            })
                            .collect()
                    })
                    .collect();
                let refs: Vec<_> = pcm.iter().map(Vec::as_slice).collect();
                let checkpoint = engine.clone();
                let result = engine.process(&frame, &refs, 48000, slots).unwrap();
                equal(
                    &result,
                    &checkpoint
                        .clone()
                        .process(&frame, &refs, 48000, slots)
                        .unwrap(),
                );
                for channel in &result {
                    assert_eq!(channel.low.len(), 2 * usize::from(slots) + 8);
                    assert_eq!(channel.high.len(), 2 * usize::from(slots) + 6);
                    if index == 0 {
                        assert!(
                            channel
                                .high
                                .iter()
                                .flatten()
                                .all(|x| *x == Complex::default())
                        );
                        assert!(channel.energy.iter().flatten().all(|x| *x == 0.0));
                    } else {
                        assert!(channel.energy.iter().flatten().any(|x| *x > 0.0));
                    }
                    for (mapped, energy) in channel.mapped.bands.iter().zip(&channel.energy) {
                        for (band, &current) in mapped.iter().zip(energy) {
                            assert_eq!(band.current, current);
                        }
                    }
                }
                // A failure after previous channels have been prepared must not
                // consume their analysis/chirp/harmonic histories.
                let mut bad = frame.clone();
                bad.parameters.channels[count - 1].envelope[0][0] = f64::NAN;
                let saved = engine.clone();
                assert!(engine.process(&bad, &refs, 48000, slots).is_err());
                assert_eq!(engine, saved);
            }
            engine.reset();
            assert_eq!(engine, Preparation::default());
        }
    }
}
