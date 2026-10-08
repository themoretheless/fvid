//! Owned non-scalable SBR extension + core PCM to normalized output PCM.
//! Extended audio dispatch and codec/container timing belong to the enclosing decoder.
use super::{
    Result, aac_sbr_downsampled_qmf,
    aac_sbr_history::{Frame, Stream},
    aac_sbr_qmf_dsp, aac_sbr_synthesis_qmf,
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
impl Synthesis {
    fn new(rate: OutputRate) -> Self {
        match rate {
            OutputRate::Double => Self::Double(Default::default()),
            OutputRate::Core => Self::Core(Default::default()),
        }
    }
    fn process(&mut self, rows: &[[super::aac_sbr_qmf::Complex; 64]]) -> Result<Vec<f64>> {
        let mut pcm = match self {
            Self::Double(s) => s.process(rows)?,
            Self::Core(s) => s.process(
                &rows
                    .iter()
                    .map(|r| r[..32].try_into().unwrap())
                    .collect::<Vec<_>>(),
            )?,
        };
        for value in &mut pcm {
            *value /= 32768.0;
        }
        Ok(pcm)
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dsp {
    qmf: aac_sbr_qmf_dsp::Dsp,
    synthesis: Vec<Synthesis>,
    output_rate: Option<OutputRate>,
}
impl Dsp {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.qmf.visit_retained(footprint)?;
        footprint.vector(&self.synthesis)
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    fn synthesize(
        &mut self,
        rows: &[Vec<[super::aac_sbr_qmf::Complex; 64]>],
        rate: OutputRate,
    ) -> Result<Vec<Vec<f64>>> {
        if self.output_rate.is_some_and(|old| old != rate) {
            return Err(invalid("SBR output rate changed without reset"));
        }
        if self.synthesis.is_empty() {
            self.synthesis
                .resize_with(rows.len(), || Synthesis::new(rate));
        }
        if self.synthesis.len() != rows.len() {
            return Err(invalid("SBR channel count changed without reset"));
        }
        let mut output = Vec::with_capacity(rows.len());
        for (state, rows) in self.synthesis.iter_mut().zip(rows) {
            output.push(state.process(rows)?);
        }
        self.output_rate = Some(rate);
        Ok(output)
    }
    pub fn process_upsampling(
        &mut self,
        pcm: &[&[f32]],
        rate: u32,
        slots: u8,
        mode: OutputRate,
    ) -> Result<Vec<Vec<f64>>> {
        let mut trial = self.clone();
        let rows = trial.qmf.process_upsampling(pcm, rate, slots)?;
        let output = trial.synthesize(&rows, mode)?;
        *self = trial;
        Ok(output)
    }
    /// One complete frame per channel. Header geometry resets preserve PCM history;
    /// format resets clear the complete pipeline. Both channels commit together.
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
            .is_some_and(|v| !v.is_empty())
        {
            return Err(unsupported(
                "SBR extended audio/PS synthesis is not yet implemented",
            ));
        }
        let mut trial = self.clone();
        let qmf = trial.qmf.process(frame, pcm, rate, slots)?;
        if qmf.format_reset {
            trial.synthesis.clear();
            trial.output_rate = None;
        }
        let output = trial.synthesize(&qmf.rows, output_rate)?;
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

#[cfg(test)]
mod upsampling_tests {
    use super::*;
    #[test]
    fn first_header_after_delay_only_nonzero_pcm_keeps_qmf_history() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("aac-sbr-dsp-oracles.json")).unwrap())
                .unwrap();
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["slots"] == 16 && c["bands"] == 64 && c["smoothing"] == true)
            .unwrap();
        let raw = std::fs::read(root.join("aac-sbr-dsp-syntax.bin")).unwrap();
        let record = &case["frames"][0];
        let at = record["offset"].as_u64().unwrap() as usize;
        let len = record["byte_length"].as_u64().unwrap() as usize;
        let mut bits = BitReader::new(&raw[at..at + len]);
        bits.skip(4).unwrap();
        let frame = Stream::default()
            .read(&mut bits, len * 8, false, 48000, 16, 1)
            .unwrap();
        assert!(frame.syntax.format_reset);
        let dense: Vec<_> = (0..1024)
            .map(|i| ((i * 73 + 19) % 257) as f32 / 256.0 - 0.5)
            .collect();
        let quiet = vec![0.0; 1024];
        let mut delayed = Dsp::default();
        delayed
            .process_upsampling(&[&dense], 48000, 16, OutputRate::Double)
            .unwrap();
        let with_history = delayed
            .process(&frame, &[&quiet], 48000, 16, OutputRate::Double)
            .unwrap();
        let fresh = Dsp::default()
            .process(&frame, &[&quiet], 48000, 16, OutputRate::Double)
            .unwrap();
        assert!(
            with_history[0]
                .iter()
                .zip(&fresh[0])
                .any(|(a, b)| (a - b).abs() > 1e-4),
            "first SBR header discarded prior QMF history"
        );
    }
    #[test]
    fn pure_upsampling_matches_direct_nonzero_time_convolution_and_rolls_back() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for slots in [15, 16] {
            for (bands, mode) in [(32, OutputRate::Core), (64, OutputRate::Double)] {
                let prefix = format!("aac-sbr-upsampling-{slots}-{bands}");
                let pcm: Vec<_> = std::fs::read(root.join(format!("{prefix}.f32le")))
                    .unwrap()
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                let expected = std::fs::read(root.join(format!("{prefix}.f64le"))).unwrap();
                for channels in [1, 2] {
                    let mut state = Dsp::default();
                    let mut output = Vec::new();
                    for block in pcm.chunks_exact(64 * usize::from(slots)) {
                        let refs = vec![block; channels];
                        let saved = state.clone();
                        let rendered = state.process_upsampling(&refs, 48000, slots, mode).unwrap();
                        let committed = state.clone();
                        state = saved;
                        assert_eq!(
                            rendered,
                            state.process_upsampling(&refs, 48000, slots, mode).unwrap()
                        );
                        assert_eq!(state, committed);
                        for channel in &rendered {
                            assert_eq!(channel, &rendered[0]);
                        }
                        output.extend_from_slice(&rendered[0]);
                        let saved = state.clone();
                        assert!(state.process_upsampling(&refs, 96000, slots, mode).is_err());
                        assert_eq!(state, saved);
                        let mut bad = block.to_vec();
                        bad[0] = f32::NAN;
                        let mut refs = vec![block; channels];
                        refs[channels - 1] = &bad;
                        assert!(state.process_upsampling(&refs, 48000, slots, mode).is_err());
                        assert_eq!(state, saved);
                    }
                    assert_eq!(expected.len(), 8 * output.len());
                    assert!(output.iter().any(|v| v.abs() > 1e-3));
                    for (&value, b) in output.iter().zip(expected.chunks_exact(8)) {
                        let reference = f64::from_le_bytes(b.try_into().unwrap());
                        assert!(
                            (value - reference).abs() < 3e-12,
                            "slots={slots} bands={bands} {value} vs {reference}"
                        );
                    }
                    state.reset();
                    assert_eq!(state, Dsp::default());
                }
            }
        }
    }
}
