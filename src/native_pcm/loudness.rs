//! Owned streaming integrated loudness; no decoder/backend dependencies.
use super::KWeighting;
use std::collections::BTreeMap;

pub use crate::media_info::PcmLoudnessStats as IntegratedLoudness;
/// Explicit channel energy weights avoid guessing layout from channel count.
/// Use 1 for front channels, 1.41 for surrounds, 0 for LFE.
/// Relative gating uses 0.01 LU histogram bins, bounding storage by level range.
pub struct LoudnessMeter {
    short_ring: Vec<f64>,
    short_position: usize,
    short_sum: f64,
    short_energies: BTreeMap<i32, (u64, f64)>,
    sample_peak: f64,
    filter: KWeighting,
    weights: Vec<f64>,
    ring: Vec<f64>,
    position: usize,
    sum: f64,
    frames: u64,
    hop: usize,
    blocks: u64,
    energies: BTreeMap<i32, (u64, f64)>,
}
impl LoudnessMeter {
    pub fn new(sample_rate: u32, weights: &[f64]) -> Result<Self, String> {
        if sample_rate % 10 != 0
            || weights
                .iter()
                .any(|w| !w.is_finite() || *w < 0.0 || *w > 2.0)
        {
            return Err(
                "loudness requires a rate divisible by ten and channel weights in 0..=2".into(),
            );
        }
        let filter = KWeighting::new(sample_rate, weights.len())?;
        Ok(Self {
            short_ring: vec![0.0; sample_rate as usize * 3],
            short_position: 0,
            short_sum: 0.0,
            short_energies: BTreeMap::new(),
            sample_peak: 0.0,
            filter,
            weights: weights.to_vec(),
            ring: vec![0.0; sample_rate as usize * 2 / 5],
            position: 0,
            sum: 0.0,
            frames: 0,
            hop: sample_rate as usize / 10,
            blocks: 0,
            energies: BTreeMap::new(),
        })
    }
    pub fn push(&mut self, pcm: &[f64]) -> Result<(), String> {
        if pcm.len() % self.weights.len() != 0
            || pcm.iter().any(|x| !x.is_finite() || x.abs() > 1e100)
        {
            return Err("loudness requires complete finite PCM frames within numeric range".into());
        }
        self.frames
            .checked_add((pcm.len() / self.weights.len()) as u64)
            .ok_or("loudness sample count overflow")?;
        let mut scratch = [0.0; 64];
        for frame in pcm.chunks_exact(self.weights.len()) {
            for sample in frame {
                self.sample_peak = self.sample_peak.max(sample.abs());
            }
            let samples = &mut scratch[..frame.len()];
            samples.copy_from_slice(frame);
            self.filter.process(samples)?;
            let energy: f64 = samples
                .iter()
                .zip(&self.weights)
                .map(|(x, w)| x * x * w)
                .sum();
            self.sum += energy - self.ring[self.position];
            self.ring[self.position] = energy;
            self.position = (self.position + 1) % self.ring.len();
            self.short_sum += energy - self.short_ring[self.short_position];
            self.short_ring[self.short_position] = energy;
            self.short_position = (self.short_position + 1) % self.short_ring.len();
            self.frames += 1;
            if self.frames >= self.short_ring.len() as u64 && self.frames % self.hop as u64 == 0 {
                let power = self.short_sum.max(0.0) / self.short_ring.len() as f64;
                let level = -0.691 + 10.0 * power.log10();
                if level >= -70.0 {
                    let entry = self
                        .short_energies
                        .entry((level * 100.0).floor() as i32)
                        .or_default();
                    entry.0 += 1;
                    entry.1 += power;
                }
            }
            if self.frames >= self.ring.len() as u64
                && (self.frames - self.ring.len() as u64) % self.hop as u64 == 0
            {
                self.blocks += 1;
                let power = self.sum.max(0.0) / self.ring.len() as f64;
                let level = -0.691 + 10.0 * power.log10();
                if level >= -70.0 {
                    let entry = self
                        .energies
                        .entry((level * 100.0).floor() as i32)
                        .or_default();
                    entry.0 += 1;
                    entry.1 += power;
                }
            }
        }
        Ok(())
    }
    pub fn report(&self) -> IntegratedLoudness {
        let count: u64 = self.energies.values().map(|e| e.0).sum();
        let total: f64 = self.energies.values().map(|e| e.1).sum();
        let threshold = if count > 0 {
            total / count as f64 / 10.0
        } else {
            f64::INFINITY
        };
        let mut gated_count = 0u64;
        let mut gated_sum = 0.0;
        for (count, sum) in self.energies.values() {
            if sum / (*count as f64) >= threshold {
                gated_count += count;
                gated_sum += sum;
            }
        }
        let short_count: u64 = self.short_energies.values().map(|e| e.0).sum();
        let short_sum: f64 = self.short_energies.values().map(|e| e.1).sum();
        let threshold = if short_count > 0 {
            short_sum / short_count as f64 / 100.0
        } else {
            f64::INFINITY
        };
        let kept: Vec<_> = self
            .short_energies
            .iter()
            .filter(|(_, e)| e.1 / e.0 as f64 >= threshold)
            .collect();
        let count: u64 = kept.iter().map(|(_, e)| e.0).sum();
        let percentile = |fraction: f64| {
            let rank = ((count as f64 * fraction).ceil() as u64).max(1);
            let mut accumulated = 0;
            for (level, e) in &kept {
                accumulated += e.0;
                if accumulated >= rank {
                    return **level as f64 / 100.0;
                }
            }
            0.0
        };
        IntegratedLoudness {
            range_lu: (count > 0).then(|| percentile(0.95) - percentile(0.1)),
            sample_peak_dbfs: (self.sample_peak > 0.0).then(|| 20.0 * self.sample_peak.log10()),
            sample_frames: self.frames,
            measured_blocks: self.blocks,
            integrated_lufs: (gated_count > 0)
                .then(|| -0.691 + 10.0 * (gated_sum / gated_count as f64).log10()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peak_is_unweighted_and_rejected_input_does_not_change_it() {
        let mut meter = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        assert!(meter.report().sample_peak_dbfs.is_none());
        meter.push(&[0.25, -0.5]).unwrap();
        assert!((meter.report().sample_peak_dbfs.unwrap() + 6.020599913279624).abs() < 1e-12);
        assert!(meter.push(&[1.0, f64::NAN]).is_err());
        assert!((meter.report().sample_peak_dbfs.unwrap() + 6.020599913279624).abs() < 1e-12);
        assert!(meter.report().integrated_lufs.is_none());
        meter.push(&[0.0, 2.0]).unwrap();
        assert!((meter.report().sample_peak_dbfs.unwrap() - 6.020599913279624).abs() < 1e-12);
    }
    #[test]
    fn silence_short_stream_chunking_and_layout_weights() {
        let mut silent = LoudnessMeter::new(48000, &[1.0]).unwrap();
        silent.push(&vec![0.0; 48000]).unwrap();
        assert!(silent.report().integrated_lufs.is_none());
        assert!(silent.report().range_lu.is_none());
        assert_eq!(silent.report().measured_blocks, 7);
        let pcm: Vec<f64> = (0..48000)
            .flat_map(|i| [(i as f64 * 0.13).sin() * 0.1, 0.0])
            .collect();
        let mut whole = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        whole.push(&pcm).unwrap();
        let mut split = LoudnessMeter::new(48000, &[1.0, 0.0]).unwrap();
        assert!(split.push(&[f64::NAN, 0.0]).is_err());
        for part in pcm.chunks(38) {
            split.push(part).unwrap();
        }
        assert_eq!(
            whole.report().integrated_lufs,
            split.report().integrated_lufs
        );
        let mut lfe = LoudnessMeter::new(48000, &[0.0, 1.0]).unwrap();
        lfe.push(&pcm).unwrap();
        assert!(lfe.report().integrated_lufs.is_none());
    }
}

#[cfg(test)]
mod oracle {
    use super::*;
    #[test]
    #[ignore = "requires FVID_REFERENCE_FFMPEG"]
    fn integrated_loudness_matches_independent_meter() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for rate in [44100, 48000, 96000] {
            let pcm: Vec<f64> = (0..rate * 18)
                .map(|i| {
                    let gain = if i < rate * 3 {
                        0.0
                    } else if i < rate * 9 {
                        0.1
                    } else {
                        0.02
                    };
                    gain * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate as f64).sin()
                })
                .collect();
            let mut meter = LoudnessMeter::new(rate, &[1.0]).unwrap();
            for part in pcm.chunks(337) {
                meter.push(part).unwrap();
            }
            let mut child = Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
                .args([
                    "-hide_banner",
                    "-nostats",
                    "-f",
                    "f64le",
                    "-ar",
                    &rate.to_string(),
                    "-ac",
                    "1",
                    "-i",
                    "pipe:0",
                    "-af",
                    "ebur128",
                    "-f",
                    "null",
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            let writer = std::thread::spawn(move || {
                for sample in pcm {
                    input.write_all(&sample.to_le_bytes()).unwrap();
                }
            });
            let result = child.wait_with_output().unwrap();
            writer.join().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let log = String::from_utf8(result.stderr).unwrap();
            let expected: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("I:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            let report = meter.report();
            let actual = report.integrated_lufs.unwrap();
            assert!(
                (actual - expected).abs() < 0.11,
                "{rate}: {actual} != {expected}"
            );
            let range: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("LRA:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (report.range_lu.unwrap() - range).abs() < 0.2,
                "{rate} LRA: {:?} != {range}",
                report.range_lu
            );
        }
    }
}

/// Analyze a selected owned audio stream without creating a decoded temporary file.
/// Channel weights must match the stream layout and are never guessed.
pub fn measure_file(
    source: &std::path::Path,
    selected: Option<usize>,
    weights: &[f64],
    cancel: Option<&crate::media_control::CancelFlag>,
) -> crate::Result<IntegratedLoudness> {
    use crate::{invalid, native_media};
    use std::io::{BufReader, Read, Seek, SeekFrom, Write};
    let mut control = native_media::DecodeProgress::new(cancel, None)?;
    let mut input = BufReader::new(std::fs::File::open(source)?);
    let mut prefix = [0u8; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    enum Input {
        Wave(BufReader<std::fs::File>, super::WaveInfo),
        Mp4(crate::container::mp4::Mp4Reader<BufReader<std::fs::File>>),
        Mka(crate::container::webm::WebmReader<BufReader<std::fs::File>>),
        Adts(crate::container::adts::StreamReader<BufReader<std::fs::File>>),
    }
    let (input, rate, channels) = if &prefix[..4] == b"RIFF" {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("WAVE has only stream 0"));
        }
        let info = super::inspect(&mut input, cancel)?;
        info.validate_decode()?;
        let (rate, channels) = (info.sample_rate, info.channels);
        (Input::Wave(input, info), rate, channels)
    } else if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open(input, Default::default())?;
        let index = native_media::mp4_audio_index(&reader, selected)?;
        let decoder = crate::native_audio_decoder::PacketPcmDecoder::new(&reader.tracks()[index])?;
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mp4(reader), rate, channels)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let reader = crate::container::webm::WebmReader::open(input, Default::default())?;
        let index = native_media::matroska_audio_index(&reader, selected)?;
        let decoder =
            crate::native_audio_decoder::PacketPcmDecoder::from_matroska(&reader.tracks[index])?;
        let (rate, channels) = (decoder.sample_rate(), decoder.channels());
        (Input::Mka(reader), rate, channels)
    } else {
        if selected.is_some_and(|s| s != 0) {
            return Err(invalid("ADTS has only stream 0"));
        }
        let reader = crate::container::adts::StreamReader::open(input)?;
        let config = reader.configuration();
        let (rate, channels) = (config.sample_rate, config.channels);
        (Input::Adts(reader), rate, channels)
    };
    if weights.len() != usize::from(channels) {
        return Err(invalid("channel weights must match selected audio stream"));
    }
    struct Sink {
        meter: LoudnessMeter,
        bytes: [u8; 4],
        byte_count: usize,
        frame: [f64; 64],
        channel: usize,
        channels: usize,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for byte in bytes {
                self.bytes[self.byte_count] = *byte;
                self.byte_count += 1;
                if self.byte_count == 4 {
                    self.byte_count = 0;
                    self.frame[self.channel] = f64::from(f32::from_le_bytes(self.bytes));
                    self.channel += 1;
                    if self.channel == self.channels {
                        self.meter
                            .push(&self.frame[..self.channels])
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                        self.channel = 0;
                    }
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        meter: LoudnessMeter::new(rate, weights).map_err(|e| invalid(&e))?,
        bytes: [0; 4],
        byte_count: 0,
        frame: [0.0; 64],
        channel: 0,
        channels: weights.len(),
    };
    let stats = match input {
        Input::Wave(reader, info) => {
            super::decode_reader(reader, info, &mut sink, None, &mut control)?
        }
        Input::Mp4(reader) => native_media::decode_mp4_audio_reader_controlled(
            reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Mka(reader) => native_media::decode_matroska_audio_reader_controlled(
            reader,
            &mut sink,
            None,
            selected,
            &mut control,
        )?,
        Input::Adts(reader) => {
            native_media::decode_adts_aac_reader_controlled(reader, &mut sink, None, &mut control)?
        }
    };
    control.check()?;
    if sink.byte_count != 0 || sink.channel != 0 || stats.sample_frames != sink.meter.frames {
        return Err(invalid("incomplete or inconsistent loudness PCM stream"));
    }
    Ok(sink.meter.report())
}

#[cfg(test)]
mod file_tests {
    #[test]
    #[ignore = "requires FVID_REFERENCE_FFMPEG"]
    fn owned_file_decoders_feed_loudness_meter() {
        use std::process::Command;
        let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
        let dir = std::env::temp_dir().join(format!("fvid-loudness-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        for (index, (codec, ext)) in [
            ("pcm_s16le", "wav"),
            ("alac", "m4a"),
            ("aac", "m4a"),
            ("aac", "mka"),
            ("aac", "aac"),
        ]
        .iter()
        .enumerate()
        {
            let source = dir.join(format!("{index}.{ext}"));
            let result = Command::new(&binary)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=1000:sample_rate=48000:duration=2",
                    "-c:a",
                    codec,
                ])
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let actual = super::measure_file(&source, None, &[1.0], None).unwrap();
            assert!(actual.sample_frames >= 96000);
            assert!(super::measure_file(&source, None, &[1.0, 1.0], None).is_err());
            let cancel = crate::media_control::CancelFlag::default();
            cancel.cancel();
            assert!(super::measure_file(&source, None, &[1.0], Some(&cancel)).is_err());
            let result = Command::new(&binary)
                .args(["-hide_banner", "-nostats", "-i"])
                .arg(&source)
                .args(["-af", "ebur128=peak=sample", "-f", "null", "-"])
                .output()
                .unwrap();
            assert!(result.status.success());
            let log = String::from_utf8(result.stderr).unwrap();
            let expected: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("I:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (actual.integrated_lufs.unwrap() - expected).abs() < 0.11,
                "{codec}/{ext}: {actual:?} vs {expected}"
            );
            let expected_peak: f64 = log
                .lines()
                .rev()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("Peak:")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap();
            assert!(
                (actual.sample_peak_dbfs.unwrap() - expected_peak).abs() < 0.11,
                "{codec}/{ext} peak: {:?} vs {expected_peak}",
                actual.sample_peak_dbfs
            );
        }
    }
}

/// Unit energy weights for conventional mono/stereo input. Multichannel callers
/// must supply explicit layout weights to `measure_loudness_file`.
pub fn default_weights(
    source: &std::path::Path,
    selected: Option<usize>,
) -> crate::Result<Vec<f64>> {
    let channels = if super::is_wave(source)? {
        if selected.is_some_and(|index| index != 0) {
            return Err(crate::invalid("WAVE has only stream 0"));
        }
        super::inspect(&mut std::fs::File::open(source)?, None)?.channels
    } else {
        crate::native_media::audio_source_info_selected(source, selected)?.channels
    };
    match channels {
        1 | 2 => Ok(vec![1.0; usize::from(channels)]),
        _ => Err(crate::invalid(
            "multichannel loudness requires explicit channel weights in stream order",
        )),
    }
}
