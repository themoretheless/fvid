use fvid_control::CopyOptions;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn same(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "sample {i}: {a} versus {b}");
    }
}
fn cases(dir: &str, file: &str) -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join(dir).join(file)).unwrap()).unwrap();
    m["cases"].as_array().unwrap().clone()
}
#[test]
fn layer1_raw_and_matroska_library_full_pcm_and_intervals_match_independent_oracles() {
    let options = CopyOptions {
        max_controlled_bytes: Some(32 << 20),
        ..Default::default()
    };
    for case in cases("mp1-native", "manifest.json") {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        let expected =
            std::fs::read(root().join("mp1-native").join(format!("{name}.f32le"))).unwrap();
        let source = root().join("mp1-native").join(format!("{name}.mp1"));
        let movie = std::fs::read(root().join("mp1-native").join(format!("{name}.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(40), Duration::from_millis(50))),
        ] {
            let wanted = if interval.is_some() {
                &expected[(rate * 40).div_ceil(1000) as usize * stride
                    ..(rate * 50).div_ceil(1000) as usize * stride]
            } else {
                &expected
            };
            let mut raw = Vec::new();
            let a = fvid_media::owned_raw_mpeg::decode_raw_mpeg_pcm(
                &source, &mut raw, interval, &options,
            )
            .unwrap_or_else(|e| panic!("{name}/raw: {e}"));
            same(&raw, wanted);
            assert_eq!(a.sample_frames, wanted.len() as u64 / stride as u64);
            let mut matroska = Vec::new();
            let b = fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                Cursor::new(&movie),
                &mut matroska,
                interval,
                &options,
            )
            .unwrap_or_else(|e| panic!("{name}/Matroska: {e}"));
            same(&matroska, wanted);
            assert_eq!(a.sample_frames, b.sample_frames);
        }
    }
}
#[test]
fn layer2_existing_owned_kernel_reaches_raw_and_matroska_library_exports() {
    let dir = root().join("mp2-native");
    let mut count = 0;
    for file in ["manifest.json", "free-manifest.json"] {
        for case in cases("mp2-native", file) {
            let name = case["name"].as_str().unwrap();
            let expected = std::fs::read(dir.join(format!("{name}.f32le"))).unwrap();
            let source = dir.join(format!("{name}.mp2"));
            let mut raw = Vec::new();
            fvid_media::owned_raw_mpeg::decode_raw_mpeg_pcm(
                &source,
                &mut raw,
                None,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}/raw: {e}"));
            same(&raw, &expected);
            if file.starts_with("free") {
                let movie = std::fs::read(dir.join(format!("{name}.mkv"))).unwrap();
                let mut pcm = Vec::new();
                fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                    Cursor::new(movie),
                    &mut pcm,
                    None,
                    &Default::default(),
                )
                .unwrap_or_else(|e| panic!("{name}/Matroska: {e}"));
                same(&pcm, &expected);
            }
            count += 1;
        }
    }
    assert!(count > 200);
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn pcm(path: &Path) -> Vec<u8> {
    let mut f = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
    assert!(info.float);
    f.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut out = vec![0; info.data_bytes as usize];
    f.read_exact(&mut out).unwrap();
    out
}
#[test]
fn public_layer1_and_layer2_plans_waves_dsp_limits_and_cancellation_are_owned() {
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-mpeg12-export-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for (stem, codec) in [
        ("mp1-native/44100-m1-e2-crc-free", "mp1"),
        ("mp2-native/free-0-0-1-48000", "mp2"),
    ] {
        for extension in [codec, "mkv"] {
            let source = root().join(format!("{stem}.{extension}"));
            let tag = format!("{codec}-{extension}");
            let plan =
                fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default())
                    .unwrap();
            assert_eq!(plan.streams[0].codec, codec);
            let out = dir.0.join(format!("{tag}.wav"));
            let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
            assert!(stats.sample_frames > 0);
            let expected = std::fs::read(root().join(format!("{stem}.f32le"))).unwrap();
            same(&pcm(&out), &expected);
            let transform = fvid_media_info::AudioDecodeTransform {
                interval: Some((1000, 45000)),
                sample_rate: Some(32000),
                channels: Some(1),
                volume: Some(0.5),
            };
            let actual = dir.0.join(format!("{tag}-actual.wav"));
            let reference = dir.0.join(format!("{tag}-reference.wav"));
            fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default())
                .unwrap();
            fvid_media::decode_audio_transformed(&out, &reference, transform, &Default::default())
                .unwrap();
            assert_eq!(pcm(&actual), pcm(&reference));
            assert!(fvid_media::decode_audio(&source, &out, &Default::default()).is_err());
            let limited = dir.0.join(format!("{tag}-budget.wav"));
            let error = fvid_media::decode_audio(
                &source,
                &limited,
                &CopyOptions {
                    max_controlled_bytes: Some(1),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                error.contains("controlled memory budget exceeded"),
                "{error}"
            );
            assert!(!limited.exists());
            let prefix = dir.0.join(format!("{tag}-prefix.wav"));
            let frames = if codec == "mp1" { 384 } else { 1152 };
            let stats = fvid_media::decode_audio(
                &source,
                &prefix,
                &CopyOptions {
                    max_packets: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.sample_frames, frames);
            same(&pcm(&prefix), &expected[..frames as usize * 2 * 4]);
            let flag = fvid_control::CancelFlag::new();
            let trigger = flag.clone();
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = events.clone();
            let cancelled = dir.0.join(format!("{tag}-cancelled.wav"));
            assert!(
                fvid_media::decode_audio(
                    &source,
                    &cancelled,
                    &CopyOptions {
                        cancel: Some(flag),
                        progress: Some(fvid_control::ProgressHook::new(move |event| {
                            captured.lock().unwrap().push(event);
                            if event.packets >= 1 {
                                trigger.cancel();
                            }
                        })),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .contains("cancelled")
            );
            assert!(!cancelled.exists());
            assert!(events.lock().unwrap().iter().all(|e| !e.done));
        }
    }
}

#[test]
fn layer1_malformed_payloads_never_publish_waves_and_eof_free_frame_is_audio() {
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-mp1-failures-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for (name, reason) in [
        ("bad-crc", "CRC mismatch"),
        ("reserved-allocation", "reserved MP1 bit allocation"),
        ("truncated", "truncated MP1 payload"),
        ("free-spacing", "length changes beyond padding"),
        ("bad-second-crc-free", "CRC mismatch"),
    ] {
        let source = root().join("mp1-native").join(format!("{name}.mkv"));
        let destination = dir.0.join(format!("{name}.wav"));
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = events.clone();
        let error = fvid_media::decode_audio(
            &source,
            &destination,
            &CopyOptions {
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    capture.lock().unwrap().push(event)
                })),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains(reason), "{name}: {error}");
        assert!(!destination.exists());
        assert!(events.lock().unwrap().iter().all(|e| !e.done));
    }
    for name in ["single-free", "free-small"] {
        let source = root().join("mp1-native").join(format!("{name}.mp1"));
        let expected =
            std::fs::read(root().join("mp1-native").join(format!("{name}.f32le"))).unwrap();
        let mut decoded = Vec::new();
        fvid_media::owned_raw_mpeg::decode_raw_mpeg_pcm(
            &source,
            &mut decoded,
            None,
            &Default::default(),
        )
        .unwrap();
        same(&decoded, &expected);
    }
    let bad = root().join("mp1-native/bad-second-crc-free.mp1");
    let destination = dir.0.join("raw-bad.wav");
    let error = fvid_media::decode_audio(&bad, &destination, &Default::default()).unwrap_err();
    assert!(error.contains("cannot infer free-format"), "{error}");
    assert!(!destination.exists());
}

#[test]
fn owned_mpeg_probe_agrees_with_the_frame_and_presentation_index() {
    for (directory, manifest, codec, extension) in [
        ("mp1-native", "manifest.json", "mp1", "mp1"),
        ("mp2-native", "manifest.json", "mp2", "mp2"),
        ("mp2-native", "free-manifest.json", "mp2", "mp2"),
        ("mp3-gapless", "manifest.json", "mp3", "mp3"),
    ] {
        for case in cases(directory, manifest) {
            let name = case["name"].as_str().unwrap();
            let source = root().join(directory).join(format!("{name}.{extension}"));
            let rate = case["rate"].as_u64().unwrap();
            let channels = case["channels"].as_u64().unwrap();
            let samples = if directory == "mp3-gapless" {
                case["end"].as_u64().unwrap() - case["start"].as_u64().unwrap()
            } else {
                std::fs::metadata(root().join(directory).join(format!("{name}.f32le")))
                    .unwrap()
                    .len()
                    / (channels * 4)
            };
            let info = fvid_media::probe(&source).unwrap_or_else(|e| panic!("{name}/probe: {e}"));
            assert_eq!(info.format, "mp3");
            assert_eq!(info.duration_us, Some((samples * 1_000_000 / rate) as i64));
            assert_eq!(info.start_us, Some(0));
            assert_eq!(info.streams.len(), 1);
            let stream = &info.streams[0];
            assert_eq!(stream.codec, codec);
            assert_eq!(stream.sample_rate, rate as i32);
            assert_eq!(stream.channels, channels as i32);
            assert_eq!(stream.duration, Some(samples as i64));
            assert_eq!(stream.time_base, [1, rate as i32]);
            let hinted = fvid_media::probe_as(&source, Some("mpeg_audio")).unwrap();
            assert_eq!(hinted.streams, info.streams);
            assert_eq!(hinted.duration_us, info.duration_us);
            assert!(fvid_media::probe_as(&source, Some("wav")).is_err());
        }
    }
    for codec in ["mp1", "mp2", "mp3"] {
        let names = fvid_media::capabilities().decoders;
        assert!(names.iter().any(|name| name == codec));
    }
    let source = root().join("mp1-native/44100-m1-e2-crc-free.mkv");
    let info = fvid_media::probe(&source).unwrap();
    assert_eq!(
        info.streams
            .iter()
            .find(|s| s.media_type == "audio")
            .unwrap()
            .codec,
        "mp1"
    );
    assert!(
        fvid_media::probe_as(&source, Some("mp1"))
            .unwrap_err()
            .contains("bare MPEG audio signature")
    );
}

#[test]
fn layer1_quantized_delay_and_padding_match_audible_pcm_windows_in_the_library() {
    let directory = root().join("mp1-native");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["clock_cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let base = case["base"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        let expected = std::fs::read(directory.join(format!("{base}.f32le"))).unwrap();
        let source = std::fs::read(directory.join(format!("{name}.mkv"))).unwrap();
        let expected = &expected[start * 8..end * 8];
        for interval in [
            None,
            Some((Duration::from_millis(40), Duration::from_millis(50))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                Cursor::new(&source),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let wanted = if interval.is_some() {
                &expected[(rate * 40).div_ceil(1000) as usize * 8
                    ..(rate * 50).div_ceil(1000) as usize * 8]
            } else {
                expected
            };
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 8);
            same(&pcm, wanted);
        }
    }
}

#[test]
fn matroska_one_tick_mpeg_gap_survives_library_export() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("mpeg-clock-metadata/manifest.json")).unwrap(),
    )
    .unwrap();
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-mpeg-tick-gap-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for layer in [2, 3] {
        let case = manifest["bases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["layer"] == layer && c["rate"] == 48000)
            .unwrap();
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        let golden = std::fs::read(root().join(case["source_pcm"].as_str().unwrap())).unwrap();
        let mut expected = golden[..1152 * stride].to_vec();
        expected.resize(expected.len() + 48 * stride, 0);
        expected.extend_from_slice(&golden[1152 * stride..]);
        let source = root().join(format!("mpeg-clock-metadata/mp{layer}-48000-tick-gap.mkv"));
        let movie = std::fs::read(&source).unwrap();
        let out = dir.0.join(format!("mp{layer}.wav"));
        fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
        same(&pcm(&out), &expected);
        for interval in [
            None,
            Some((Duration::from_millis(24), Duration::from_millis(26))),
        ] {
            let wanted = if interval.is_some() {
                &expected[1152 * stride..1248 * stride]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                Cursor::new(&movie),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            same(&pcm, wanted);
            assert_eq!(stats.sample_frames, wanted.len() as u64 / stride as u64);
        }
    }
}
