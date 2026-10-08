use fvid_media::{CopyOptions, ProgressHook, owned_aac::decode_adts_pcm};
use std::{
    cell::Cell,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn pairs() -> [(&'static str, &'static str, u16); 3] {
    [
        ("he-aac-implicit-sbr.aac", "he-aac-implicit-sbr.mp4", 1),
        ("he-aac-delayed-sbr.aac", "he-aac-delayed-sbr.mp4", 1),
        (
            "he-aac-implicit-sbr-stereo.aac",
            "he-aac-implicit-sbr-stereo.mp4",
            2,
        ),
    ]
}
fn video_pcm(name: &str) -> Vec<u8> {
    let bytes = std::fs::read(fixture(name)).unwrap();
    let mut output = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!((stats.sample_rate, stats.sample_frames), (48000, 6144));
    output
}
struct Short {
    source: Cursor<Vec<u8>>,
    count: Rc<Cell<usize>>,
}
impl Read for Short {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let length = out.len().min(3);
        let read = self.source.read(&mut out[..length])?;
        self.count.set(self.count.get() + read);
        Ok(read)
    }
}

#[test]
fn negotiated_adts_matches_original_video_and_independent_pcm_including_delayed_fil() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("he-aac-implicit-sbr.json")).unwrap())
            .unwrap();
    for (adts, video, channels) in pairs() {
        let bytes = std::fs::read(fixture(adts)).unwrap();
        let video_bytes = std::fs::read(fixture(video)).unwrap();
        let mut video_reader =
            fvid::playback_native::NativeReader::software(Cursor::new(video_bytes), usize::MAX)
                .unwrap();
        assert!(
            video_reader.read_frame_raw().unwrap().is_some(),
            "paired original video must play"
        );
        let expected = video_pcm(video);
        let (record, pcm_file) = if channels == 2 {
            (&manifest["stereo"]["adts"], "aac-sbr-dsp-pcm.f64le")
        } else if adts.contains("delayed") {
            (&manifest["delayed"]["adts"], "he-aac-missing-sbr.f64le")
        } else {
            (&manifest["adts"], "aac-sbr-dsp-pcm.f64le")
        };
        let oracle = std::fs::read(fixture(pcm_file)).unwrap();
        let offset = record["pcm_offset"].as_u64().unwrap() as usize;
        let frames = record["samples"].as_u64().unwrap() as usize;
        assert_eq!(frames, 6144);
        for (frame, raw) in expected
            .chunks_exact(usize::from(channels) * 4)
            .zip(oracle[offset..offset + frames * 8].chunks_exact(8))
        {
            let value = f64::from_le_bytes(raw.try_into().unwrap()) as f32;
            for channel in frame.chunks_exact(4) {
                assert_eq!(
                    f32::from_le_bytes(channel.try_into().unwrap()).to_bits(),
                    value.to_bits()
                );
            }
        }
        // Exact old failure: core-only strict decoding succeeds on the initial
        // no-FIL block, then refuses the valid SBR FIL (never a demux error).
        let mut reader = fvid_media::owned_aac::adts::StreamReader::open(bytes.as_slice()).unwrap();
        assert_eq!(reader.configuration().sample_rate, 24000);
        let mut strict =
            fvid_media::owned_aac::NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
        if adts.contains("delayed") {
            assert_eq!(
                strict
                    .decode(&reader.next_packet().unwrap().unwrap())
                    .unwrap()
                    .len(),
                1024
            );
        }
        let error = strict
            .decode(&reader.next_packet().unwrap().unwrap())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("SBR requires extension-aware stream signalling"),
            "{error}"
        );
        let options = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        let mut pcm = Vec::new();
        let stats = decode_adts_pcm(bytes.as_slice(), &mut pcm, None, &options).unwrap();
        assert_eq!(
            (
                stats.sample_rate,
                stats.channels,
                stats.sample_frames,
                stats.decoded_frames
            ),
            (48000, channels, 6144, 3)
        );
        assert_eq!(pcm, expected);
        let mut sequential = Vec::new();
        let reader = fvid::container::adts::StreamReader::open(bytes.as_slice()).unwrap();
        let core =
            fvid::native_media::decode_adts_aac_reader(reader, &mut sequential, None).unwrap();
        assert_eq!((core.sample_rate, core.sample_frames), (48000, 6144));
        assert_eq!(sequential, expected);
        let mut indexed = Vec::new();
        let core =
            fvid::native_media::decode_aac_pcm(&bytes, &mut indexed, &Default::default()).unwrap();
        assert_eq!((core.sample_rate, core.sample_frames), (48000, 6144));
        assert_eq!(indexed, expected);
    }
}

#[test]
fn packet_and_fractional_range_limits_preserve_negotiated_prefix_and_leave_tail_unread() {
    for (adts, video, channels) in pairs() {
        let bytes = std::fs::read(fixture(adts)).unwrap();
        let expected = video_pcm(video);
        let mut lengths = vec![0];
        let mut at = 0;
        while at < bytes.len() {
            at += fvid_media::owned_aac::adts::header(&bytes[at..])
                .unwrap()
                .frame_bytes;
            lengths.push(at);
        }
        let mut truncated = bytes.clone();
        truncated.extend_from_slice(&[0xff, 0xf1]);
        for packets in 1..=3 {
            let count = Rc::new(Cell::new(0));
            let source = Short {
                source: Cursor::new(truncated.clone()),
                count: count.clone(),
            };
            let mut output = Vec::new();
            let options = CopyOptions {
                max_packets: Some(packets as u64),
                ..Default::default()
            };
            let stats = decode_adts_pcm(source, &mut output, None, &options).unwrap();
            assert_eq!(count.get(), lengths[packets]);
            let delayed_core = adts.contains("delayed") && packets == 1;
            let rate = if delayed_core { 24000 } else { 48000 };
            let frames = packets * if delayed_core { 1024 } else { 2048 };
            assert_eq!(
                (stats.sample_rate, stats.sample_frames, stats.decoded_frames),
                (rate, frames as u64, packets as u64)
            );
            assert_eq!(output, &expected[..frames * usize::from(channels) * 4]);
        }
        for (from, to) in [(12_345_678, 65_432_101), (65_432_101, 127_999_999)] {
            let count = Rc::new(Cell::new(0));
            let source = Short {
                source: Cursor::new(truncated.clone()),
                count: count.clone(),
            };
            let interval = Some((Duration::from_nanos(from), Duration::from_nanos(to)));
            let mut output = Vec::new();
            let stats =
                decode_adts_pcm(source, &mut output, interval, &Default::default()).unwrap();
            let first = (u128::from(from) * 48000).div_ceil(1_000_000_000) as usize;
            let last = (u128::from(to) * 48000).div_ceil(1_000_000_000) as usize;
            let stride = usize::from(channels) * 4;
            assert_eq!(stats.sample_rate, 48000);
            assert_eq!(stats.sample_frames, (last - first) as u64);
            assert_eq!(output, &expected[first * stride..last * stride]);
            assert!(count.get() <= bytes.len());
            if to > 85_333_334 {
                assert_eq!(count.get(), bytes.len());
            }
            let mut core = Vec::new();
            let reader = fvid::container::adts::StreamReader::open(truncated.as_slice()).unwrap();
            fvid::native_media::decode_adts_aac_reader(reader, &mut core, interval).unwrap();
            assert_eq!(core, output);
        }
        assert!(
            decode_adts_pcm(
                truncated.as_slice(),
                &mut Vec::new(),
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string()
            .contains("fill whole buffer")
        );
    }
}

#[test]
fn discovery_admission_and_progress_charge_sbr_storage_and_count_source_packets_once() {
    for (adts, _, channels) in pairs() {
        let bytes = std::fs::read(fixture(adts)).unwrap();
        let count = Rc::new(Cell::new(0));
        let source = Short {
            source: Cursor::new(bytes.clone()),
            count: count.clone(),
        };
        let options = CopyOptions {
            max_controlled_bytes: Some(7 * 1024 * 1024),
            ..Default::default()
        };
        let mut output = Vec::new();
        let error = decode_adts_pcm(source, &mut output, None, &options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded")
        );
        assert!(output.is_empty());
        assert_eq!(count.get(), 7, "admission before packet payload");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let options = CopyOptions {
            progress: Some(ProgressHook::new(move |e| captured.lock().unwrap().push(e))),
            ..Default::default()
        };
        let stats = decode_adts_pcm(bytes.as_slice(), &mut output, None, &options).unwrap();
        assert_eq!(stats.channels, channels);
        let events = events.lock().unwrap();
        assert_eq!(
            events.iter().map(|e| e.packets).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            events.last().unwrap().payload_bytes as usize,
            bytes.len() - 21
        );
        assert!(events.iter().all(|e| !e.done));
    }
}

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn file_exports_choose_sbr_clock_before_wav_headers_resampling_and_trimming() {
    let temp =
        Temp(std::env::temp_dir().join(format!("fvid-adts-sbr-export-{}", std::process::id())));
    std::fs::create_dir_all(&temp.0).unwrap();
    for (adts, video, channels) in pairs() {
        let source = fixture(adts);
        let expected_pcm = video_pcm(video);
        let reference = temp.0.join(format!("{adts}-reference.wav"));
        let mut data = fvid_media::owned_wav::float_wav_header_with_mask(
            48000,
            channels,
            6144,
            if channels == 1 { 4 } else { 3 },
        )
        .unwrap();
        data.extend_from_slice(&expected_pcm);
        std::fs::write(&reference, data).unwrap();
        for (index, transform) in [
            fvid_media_info::AudioDecodeTransform::default(),
            fvid_media_info::AudioDecodeTransform {
                interval: Some((12345, 98765)),
                volume: Some(0.5),
                ..Default::default()
            },
            fvid_media_info::AudioDecodeTransform {
                interval: Some((12345, 98765)),
                sample_rate: Some(16000),
                channels: Some(1),
                ..Default::default()
            },
            // The requested rate matches the ADTS core, but SBR output still
            // needs resampling lookahead for a range that ends inside a packet.
            fvid_media_info::AudioDecodeTransform {
                interval: Some((12345, 40000)),
                sample_rate: Some(24000),
                ..Default::default()
            },
        ]
        .into_iter()
        .enumerate()
        {
            let actual = temp.0.join(format!("{adts}-{index}-owned.wav"));
            let expected = temp.0.join(format!("{adts}-{index}-reference.wav"));
            fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default())
                .unwrap();
            fvid_media::decode_audio_transformed(
                &reference,
                &expected,
                transform,
                &Default::default(),
            )
            .unwrap();
            assert!(
                std::fs::read(actual).unwrap() == std::fs::read(expected).unwrap(),
                "PCM/header mismatch: {adts} transform {index}"
            );
        }
        let raw = temp.0.join(format!("{adts}.f32le"));
        let stats = fvid::native_export::export_aac_pcm(&source, &raw).unwrap();
        assert_eq!(
            (stats.sample_rate, stats.sample_frames, stats.channels),
            (48000, 6144, channels)
        );
        assert_eq!(std::fs::read(raw).unwrap(), expected_pcm);
        let wav = temp.0.join(format!("{adts}-core.wav"));
        let stats = fvid::native_export::export_aac_pcm(&source, &wav).unwrap();
        assert_eq!(stats.sample_rate, 48000);
        assert_eq!(
            std::fs::read(wav).unwrap(),
            std::fs::read(&reference).unwrap()
        );
    }
}

#[test]
fn source_info_plans_loudness_and_normalization_use_the_discovered_output_clock() {
    let temp =
        Temp(std::env::temp_dir().join(format!("fvid-adts-sbr-analysis-{}", std::process::id())));
    std::fs::create_dir_all(&temp.0).unwrap();
    for (adts, video, channels) in pairs() {
        let source = fixture(adts);
        let info = fvid::native_media::aac_source_info(&source).unwrap();
        assert_eq!((info.sample_rate, info.channels), (48000, channels));
        let options = CopyOptions {
            progress: Some(ProgressHook::new(|_| {
                panic!("read-only plan emitted progress")
            })),
            ..Default::default()
        };
        let plan = fvid_media::plan_decode_audio(
            &source,
            &fvid_media_info::AudioDecodeTransform {
                sample_rate: Some(24000),
                ..Default::default()
            },
            &options,
        )
        .unwrap();
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == "resample" && step.detail.contains("48000 → 24000"))
        );
        let plan = fvid_media::plan_decode_audio(
            &source,
            &fvid_media_info::AudioDecodeTransform {
                sample_rate: Some(48000),
                ..Default::default()
            },
            &options,
        )
        .unwrap();
        assert!(!plan.steps.iter().any(|step| step.action == "resample"));
        let reference = temp.0.join(format!("{adts}-reference.wav"));
        let mut data = fvid_media::owned_wav::float_wav_header_with_mask(
            48000,
            channels,
            6144,
            if channels == 1 { 4 } else { 3 },
        )
        .unwrap();
        data.extend(video_pcm(video));
        std::fs::write(&reference, data).unwrap();
        let mut expected =
            fvid_media::owned_wave_loudness::measure_loudness(&reference, &Default::default())
                .unwrap();
        expected.backend = "owned ADTS AAC loudness";
        let actual = fvid_media::measure_loudness(&source, &Default::default()).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        for dual in [false, true] {
            let actual = temp.0.join(format!("{adts}-{dual}-actual.wav"));
            let expected = temp.0.join(format!("{adts}-{dual}-expected.wav"));
            if dual {
                fvid_media::apply_loudnorm_dual(&source, &actual, None, &Default::default())
                    .unwrap();
                fvid_media::owned_loudnorm::apply_loudnorm_dual(
                    &reference,
                    &expected,
                    None,
                    &Default::default(),
                )
                .unwrap();
            } else {
                fvid_media::apply_loudnorm(&source, &actual, None, &Default::default()).unwrap();
                fvid_media::owned_loudnorm::apply_loudnorm(
                    &reference,
                    &expected,
                    None,
                    &Default::default(),
                )
                .unwrap();
            }
            assert!(
                std::fs::read(actual).unwrap() == std::fs::read(expected).unwrap(),
                "loudnorm mismatch: {adts} dual={dual}"
            );
        }
    }
}
