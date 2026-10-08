//! Owned non-scalable SBR extension + core PCM to normalized output PCM.
//! PS, core AAC element dispatch and codec/container timing belong to the
//! enclosing decoder; this module does not silently interpret PS as stereo.
use super::{
    Result,
    aac_sbr_assembly::Assembly,
    aac_sbr_buffers::SynthesisRows,
    aac_sbr_downsampled_qmf, aac_sbr_gain, aac_sbr_hf,
    aac_sbr_history::{Frame, Stream},
    aac_sbr_limiter,
    aac_sbr_prepare::Preparation,
    aac_sbr_synthesis_qmf,
    bits::BitReader,
    invalid, unsupported,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputRate {
    Double,
    Core,
}
#[derive(Clone, Debug, PartialEq)]
enum Synthesis {
    Double(aac_sbr_synthesis_qmf::Synthesis),
    Core(aac_sbr_downsampled_qmf::Synthesis),
}
#[derive(Clone, Debug, PartialEq)]
struct Channel {
    assembly: Assembly,
    rows: SynthesisRows,
    synthesis: Synthesis,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dsp {
    preparation: Preparation,
    channels: Vec<Channel>,
    output_rate: Option<OutputRate>,
}
impl Dsp {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// One complete frame per channel. Values remain unclipped normalized f64;
    /// the caller owns interleaving, final sample format and timestamp policy.
    /// Header geometry reset preserves synthesis/overlap, while full format
    /// reset or seek clears the complete stream. Output-rate changes require
    /// full reset, because the two synthesis banks have different histories.
    pub fn process(
        &mut self,
        frame: &Frame,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<Vec<Vec<f64>>> {
        if frame
            .syntax
            .data
            .extended_data
            .as_ref()
            .is_some_and(|bytes| !bytes.is_empty())
        {
            return Err(unsupported(
                "SBR extended audio/PS synthesis is not yet implemented",
            ));
        }
        let mut trial = self.clone();
        if frame.syntax.format_reset {
            trial.reset();
        }
        if trial.output_rate.is_some_and(|old| old != output_rate) {
            return Err(invalid("SBR output rate changed without reset"));
        }
        let prepared = trial.preparation.process(frame, pcm, rate, slots)?;
        if trial.channels.is_empty() {
            for _ in &prepared {
                trial.channels.push(Channel {
                    assembly: Assembly::default(),
                    rows: SynthesisRows::default(),
                    synthesis: match output_rate {
                        OutputRate::Double => {
                            Synthesis::Double(aac_sbr_synthesis_qmf::Synthesis::default())
                        }
                        OutputRate::Core => {
                            Synthesis::Core(aac_sbr_downsampled_qmf::Synthesis::default())
                        }
                    },
                });
            }
        }
        let tables = &frame.syntax.data.frequency;
        let header = &frame.syntax.header;
        let kx = tables.high[0];
        let end = *tables.high.last().unwrap();
        let patches = aac_sbr_hf::patches(&tables.master, kx, rate)?;
        let borders = aac_sbr_limiter::borders(&tables.low, &patches, header.limiter_bands)?;
        let mut output = Vec::with_capacity(prepared.len());
        for (index, prepared) in prepared.into_iter().enumerate() {
            let state = &mut trial.channels[index];
            let grid = frame.syntax.data.channels[index].grid.time_grid(slots)?;
            let mut levels = Vec::with_capacity(prepared.mapped.bands.len());
            for (bands, &suppress) in prepared
                .mapped
                .bands
                .iter()
                .zip(&prepared.mapped.suppress_noise)
            {
                let initial = aac_sbr_gain::calculate(bands, suppress)?;
                levels.push(aac_sbr_gain::limit(
                    bands,
                    &initial,
                    kx,
                    &borders,
                    header.limiter_gains,
                    suppress,
                )?);
            }
            let adjusted = state.assembly.process(
                &prepared.high,
                slots,
                &grid,
                kx,
                &levels,
                &prepared.mapped.suppress_noise,
                header.smoothing_mode,
                frame.syntax.header_reset,
            )?;
            let rows = state
                .rows
                .process(&prepared.low, &adjusted, slots, &grid, kx, end)?;
            let mut channel = match &mut state.synthesis {
                Synthesis::Double(synthesis) => synthesis.process(&rows)?,
                Synthesis::Core(synthesis) => {
                    let low: Vec<[super::aac_sbr_qmf::Complex; 32]> =
                        rows.iter().map(|r| r[..32].try_into().unwrap()).collect();
                    synthesis.process(&low)?
                }
            };
            for value in &mut channel {
                *value /= 32768.0;
            }
            output.push(channel);
        }
        trial.output_rate = Some(output_rate);
        *self = trial;
        Ok(output)
    }
}
/// Reader/header/coefficient and all channel DSP histories form one transaction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoder {
    stream: Stream,
    dsp: Dsp,
}
impl Decoder {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn read(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        crc: bool,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<Vec<Vec<f64>>> {
        let mut state = self.clone();
        let mut trial = bits.clone();
        let frame = state
            .stream
            .read(&mut trial, end, crc, rate, slots, pcm.len())?;
        let output = state.dsp.process(&frame, pcm, rate, slots, output_rate)?;
        *self = state;
        *bits = trial;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    const SYNTAX: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-dsp-syntax.bin");
    const PCM: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
    fn fixture() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-dsp-oracles.json"
        ))
        .unwrap()
    }
    fn bytes(frame: &Value) -> &[u8] {
        let offset = frame["offset"].as_u64().unwrap() as usize;
        &SYNTAX[offset..offset + frame["byte_length"].as_u64().unwrap() as usize]
    }
    fn open(raw: &[u8]) -> (BitReader<'_>, bool) {
        let mut bits = BitReader::new(raw);
        let kind = bits.read(4).unwrap();
        (bits, kind == 14)
    }
    #[test]
    fn original_sbr_payloads_synthesize_pcm_against_independent_noise_gain_convolution() {
        let manifest = fixture();
        for case in manifest["cases"].as_array().unwrap() {
            let slots = case["slots"].as_u64().unwrap() as u8;
            let bands = case["bands"].as_u64().unwrap() as usize;
            let rate = if bands == 64 {
                OutputRate::Double
            } else {
                OutputRate::Core
            };
            let core = vec![0.0; 64 * usize::from(slots)];
            let mut decoder = Decoder::default();
            let mut output = Vec::new();
            for frame in case["frames"].as_array().unwrap() {
                let raw = bytes(frame);
                let (mut bits, crc) = open(raw);
                let checkpoint = decoder.clone();
                let value = decoder
                    .read(&mut bits, raw.len() * 8, crc, &[&core], 48000, slots, rate)
                    .unwrap();
                assert_eq!(bits.remaining(), 0);
                assert_eq!(value.len(), 1);
                assert_eq!(value[0].len(), 2 * usize::from(slots) * bands);
                let (mut replay, crc) = open(raw);
                assert_eq!(
                    value,
                    checkpoint
                        .clone()
                        .read(
                            &mut replay,
                            raw.len() * 8,
                            crc,
                            &[&core],
                            48000,
                            slots,
                            rate
                        )
                        .unwrap()
                );
                output.extend_from_slice(&value[0]);
            }
            assert_eq!(output.len(), case["samples"].as_u64().unwrap() as usize);
            let offset = case["pcm_offset"].as_u64().unwrap() as usize;
            for (&value, expected) in output
                .iter()
                .zip(PCM[offset..offset + 8 * output.len()].chunks_exact(8))
            {
                let expected = f64::from_le_bytes(expected.try_into().unwrap());
                assert!(
                    (value - expected).abs() < 2e-14,
                    "slots={slots} bands={bands} mode={} smoothing={} {value} vs {expected}",
                    case["limiter"],
                    case["smoothing"]
                );
            }
            assert!(output.iter().any(|v| v.abs() > 1e-5));
            decoder.reset();
            assert_eq!(decoder, Decoder::default());
        }
    }
    #[test]
    fn parsed_headers_coefficients_reader_and_dsp_rollback_together_on_late_failure() {
        let manifest = fixture();
        for case in manifest["cases"].as_array().unwrap() {
            let slots = case["slots"].as_u64().unwrap() as u8;
            let rate = if case["bands"] == 64 {
                OutputRate::Double
            } else {
                OutputRate::Core
            };
            let core = vec![0.0; 64 * usize::from(slots)];
            let mut decoder = Decoder::default();
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = bytes(frame);
                let saved = decoder.clone();
                let mut bad_core = core.clone();
                *bad_core.last_mut().unwrap() = f32::NAN;
                let (mut bits, crc) = open(raw);
                let start = bits.position();
                assert!(
                    decoder
                        .read(
                            &mut bits,
                            raw.len() * 8,
                            crc,
                            &[&bad_core],
                            48000,
                            slots,
                            rate
                        )
                        .is_err()
                );
                assert_eq!(bits.position(), start);
                assert_eq!(decoder, saved);
                if index > 0 {
                    let other = if rate == OutputRate::Double {
                        OutputRate::Core
                    } else {
                        OutputRate::Double
                    };
                    assert!(
                        decoder
                            .read(&mut bits, raw.len() * 8, crc, &[&core], 48000, slots, other)
                            .is_err()
                    );
                    assert_eq!(bits.position(), start);
                    assert_eq!(decoder, saved);
                }
                let (mut bits, crc) = open(raw);
                decoder
                    .read(&mut bits, raw.len() * 8, crc, &[&core], 48000, slots, rate)
                    .unwrap();
            }
        }
    }
    #[test]
    fn mono_and_coupled_uncoupled_stereo_reach_finite_pcm_across_all_grid_classes() {
        let binary =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-history-syntax.bin");
        let manifest: Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-history-decimal.json"
        ))
        .unwrap();
        for case in manifest["sequences"].as_array().unwrap() {
            let slots = case["slots"].as_u64().unwrap() as u8;
            let count = if case["kind"] == "mono" { 1 } else { 2 };
            for rate in [OutputRate::Double, OutputRate::Core] {
                let mut decoder = Decoder::default();
                for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                    let offset = frame["offset"].as_u64().unwrap() as usize;
                    let raw =
                        &binary[offset..offset + frame["byte_length"].as_u64().unwrap() as usize];
                    let pcm: Vec<Vec<f32>> = (0..count)
                        .map(|c| {
                            (0..64 * usize::from(slots))
                                .map(|i| (((i * 17 + c * 7 + index * 3) % 31) as f32 - 15.0) / 64.0)
                                .collect()
                        })
                        .collect();
                    let refs: Vec<_> = pcm.iter().map(Vec::as_slice).collect();
                    let (mut bits, crc) = open(raw);
                    let checkpoint = decoder.clone();
                    let output = decoder
                        .read(&mut bits, raw.len() * 8, crc, &refs, 48000, slots, rate)
                        .unwrap();
                    assert_eq!(bits.remaining(), 0);
                    assert_eq!(output.len(), count);
                    let length =
                        usize::from(slots) * if rate == OutputRate::Double { 128 } else { 64 };
                    assert!(
                        output.iter().all(|channel| channel.len() == length
                            && channel.iter().all(|x| x.is_finite()))
                    );
                    let (mut replay, crc) = open(raw);
                    assert_eq!(
                        output,
                        checkpoint
                            .clone()
                            .read(&mut replay, raw.len() * 8, crc, &refs, 48000, slots, rate)
                            .unwrap()
                    );
                    // A late typed second-channel failure reaches DSP mapping,
                    // after prior-channel preparation, and rolls it all back.
                    let mut stream = Stream::default();
                    // Recreate syntax history through this sequence prefix.
                    let mut parsed = None;
                    for earlier in &case["frames"].as_array().unwrap()[..=index] {
                        let off = earlier["offset"].as_u64().unwrap() as usize;
                        let raw =
                            &binary[off..off + earlier["byte_length"].as_u64().unwrap() as usize];
                        let (mut b, crc) = open(raw);
                        parsed = Some(
                            stream
                                .read(&mut b, raw.len() * 8, crc, 48000, slots, count)
                                .unwrap(),
                        );
                    }
                    let mut bad = parsed.unwrap();
                    bad.parameters.channels[count - 1].envelope[0][0] = f64::NAN;
                    let mut dsp = checkpoint.dsp.clone();
                    let saved = dsp.clone();
                    assert!(dsp.process(&bad, &refs, 48000, slots, rate).is_err());
                    assert_eq!(dsp, saved);
                    bad.parameters.channels[count - 1].envelope[0][0] = 64.0;
                    bad.syntax.data.extended_data = Some(vec![0x80]);
                    let error = dsp.process(&bad, &refs, 48000, slots, rate).unwrap_err();
                    assert!(error.to_string().contains("extended audio/PS"));
                    assert_eq!(dsp, saved);
                }
            }
        }
    }
}
