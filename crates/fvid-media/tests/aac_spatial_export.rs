use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn pcm(path: &Path) -> Vec<f32> {
    let mut f = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
    f.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut bytes = vec![0; info.data_bytes as usize];
    f.read_exact(&mut bytes).unwrap();
    bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn public_aac_spatial_export_matches_independent_pce_oracles_in_every_container() {
    let root = root();
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-aac-spatial-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as i32;
        let golden =
            std::fs::read(root.join(format!("aac-device-layout/{name}.stereo.f32le"))).unwrap();
        let stereo: Vec<f32> = golden
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        for output in [1, 2] {
            if output == channels {
                continue;
            }
            let expected: Vec<f32> = if output == 1 {
                stereo
                    .chunks_exact(2)
                    .map(|p| (p[0] + p[1]) * 0.5)
                    .collect()
            } else {
                stereo.clone()
            };
            for ext in ["aac", "mkv", "mp4"] {
                let source = if ext == "aac" {
                    root.join(format!("aac-height-{name}.aac"))
                } else {
                    root.join(format!("aac-device-layout/{name}.{ext}"))
                };
                let out = dir.join(format!("{name}-{ext}-{output}.wav"));
                let transform = fvid_media_info::AudioDecodeTransform {
                    channels: Some(output),
                    ..Default::default()
                };
                fvid_media::plan_decode_audio(&source, &transform, &Default::default())
                    .unwrap_or_else(|e| panic!("{name}/{ext}: {e}"));
                let stats = fvid_media::decode_audio_transformed(
                    &source,
                    &out,
                    transform,
                    &Default::default(),
                )
                .unwrap_or_else(|e| panic!("{name}/{ext}: {e}"));
                assert_eq!(stats.sample_frames, 6144);
                let actual = pcm(&out);
                assert_eq!(actual.len(), expected.len(), "{name}/{ext}");
                let mut square = 0f64;
                let mut peak = 0f32;
                for (&a, &b) in actual.iter().zip(&expected) {
                    let d = (a - b).abs();
                    peak = peak.max(d);
                    square += f64::from(d).powi(2);
                }
                assert!(
                    peak < 2e-6 && (square / actual.len() as f64).sqrt() < 4e-7,
                    "{name}/{ext}/ch{output}: peak{peak}"
                );
            }
        }
    }
}
#[test]
fn pce_intervals_resampling_and_gain_match_independent_stereo_wave_controls() {
    let root = root();
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-aac-spatial-dsp-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as i32;
        let golden =
            std::fs::read(root.join(format!("aac-device-layout/{name}.stereo.f32le"))).unwrap();
        let mut wave =
            fvid_media::owned_wav::float_wav_header_with_mask(48000, 2, 6144, 3).unwrap();
        wave.extend(golden);
        let control = dir.join(format!("{name}-control.wav"));
        std::fs::write(&control, wave).unwrap();
        for output in [1, 2] {
            if output == channels {
                continue;
            }
            let transform = fvid_media_info::AudioDecodeTransform {
                channels: Some(output),
                sample_rate: Some(32000),
                interval: Some((4000, 70000)),
                volume: Some(0.5),
            };
            let reference = dir.join(format!("{name}-{output}-reference.wav"));
            fvid_media::decode_audio_transformed(
                &control,
                &reference,
                transform,
                &Default::default(),
            )
            .unwrap();
            let expected = pcm(&reference);
            for ext in ["aac", "mkv", "mp4"] {
                let source = if ext == "aac" {
                    root.join(format!("aac-height-{name}.aac"))
                } else {
                    root.join(format!("aac-device-layout/{name}.{ext}"))
                };
                let out = dir.join(format!("{name}-{ext}-{output}.wav"));
                let stats = fvid_media::decode_audio_transformed(
                    &source,
                    &out,
                    transform,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(stats.sample_frames, 2112);
                let actual = pcm(&out);
                assert_eq!(actual.len(), expected.len());
                for (&a, &b) in actual.iter().zip(&expected) {
                    assert!((a - b).abs() < 2e-6, "{name}/{ext}: {a} vs {b}");
                }
            }
        }
    }
}
#[test]
fn ps_adts_spool_uses_negotiated_stereo_geometry_and_preserves_pcm() {
    let root = root();
    let dir = std::env::temp_dir().join(format!("fvid-ps-spool-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for name in ["immediate", "late"] {
        let source = root.join(format!("aac-main-ps-adts-{name}-synthetic.aac"));
        let mut expected = Vec::new();
        let native = fvid_media::owned_aac::decode_adts_pcm(
            std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(native.channels, 2);
        let out = dir.join(format!("{name}.wav"));
        let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&out).unwrap(), None)
                .unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(info.channel_mask, 3);
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(stats.sample_frames, native.sample_frames);
        let actual: Vec<u8> = pcm(&out).into_iter().flat_map(f32::to_le_bytes).collect();
        assert_eq!(actual, expected, "{name}");
        let mono = dir.join(format!("{name}-mono.wav"));
        fvid_media::decode_audio_transformed(
            &source,
            &mono,
            fvid_media_info::AudioDecodeTransform {
                channels: Some(1),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let stereo: Vec<f32> = expected
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let wanted: Vec<_> = stereo
            .chunks_exact(2)
            .map(|p| (p[0] + p[1]) * 0.5)
            .collect();
        assert_eq!(pcm(&mono), wanted, "{name} mono");
    }
}

#[test]
fn spatial_export_controls_preserve_prefixes_and_atomic_publication() {
    use fvid_control::{CancelFlag, CopyOptions, ProgressHook};
    use fvid_media_info::AudioDecodeTransform;
    let dir = TestDir::new("controls");
    let root = root();
    let golden: Vec<f32> = std::fs::read(root.join("aac-device-layout/normal-ten.stereo.f32le"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let transform = AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    for ext in ["aac", "mp4", "mkv"] {
        let source = if ext == "aac" {
            root.join("aac-height-normal-ten.aac")
        } else {
            root.join(format!("aac-device-layout/normal-ten.{ext}"))
        };
        let out = dir.path().join(format!("prefix-{ext}.wav"));
        let stats = fvid_media::decode_audio_transformed(
            &source,
            &out,
            transform,
            &CopyOptions {
                max_packets: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 1024, "{ext}");
        let actual = pcm(&out);
        assert_eq!(actual.len(), 2048);
        for (a, b) in actual.iter().zip(&golden) {
            assert!((a - b).abs() < 2e-6, "{ext}");
        }
        for mode in 0..3 {
            let out = dir.path().join(format!("refuse-{ext}-{mode}.wav"));
            let cancel = CancelFlag::new();
            let captured = cancel.clone();
            let mut options = CopyOptions::default();
            if mode == 0 {
                options.max_controlled_bytes = Some(1);
            }
            if mode == 1 {
                options.cancel = Some(cancel);
                options.progress = Some(ProgressHook::new(move |event| {
                    assert!(!event.done);
                    if event.packets > 0 {
                        captured.cancel();
                    }
                }));
            }
            if mode == 2 {
                std::fs::write(&out, b"preserved").unwrap();
            }
            let error = fvid_media::decode_audio_transformed(&source, &out, transform, &options)
                .unwrap_err();
            assert!(
                error.contains(match mode {
                    0 => "controlled memory",
                    1 => "cancel",
                    _ => "exists",
                }),
                "{ext}/{mode}: {error}"
            );
            if mode == 2 {
                assert_eq!(std::fs::read(&out).unwrap(), b"preserved");
            } else {
                assert!(!out.exists());
            }
        }
    }
}

#[test]
fn public_aac_export_quantization_gap_intervals_and_overlap_policy() {
    use fvid_media_info::AudioDecodeTransform;
    let dir = TestDir::new("clock");
    let golden: Vec<f32> = std::fs::read(root().join("aac-device-layout/normal-ten.stereo.f32le"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    for name in ["coarse", "gap"] {
        let mut expected = golden.clone();
        if name == "gap" {
            expected.splice(2048 * 2..2048 * 2, std::iter::repeat_n(0., 4768 * 2));
        }
        for interval in [None, Some((50_000, 180_000))] {
            // Coarse fixture ends at128ms; end is clamped by source PCM.
            let begin = if interval.is_some() { 2400 } else { 0 };
            let end = if interval.is_some() {
                8640.min(expected.len() / 2)
            } else {
                expected.len() / 2
            };
            let out = dir
                .path()
                .join(format!("{name}-{}.wav", interval.is_some()));
            let stats = fvid_media::decode_audio_transformed(
                &root().join(format!("aac-clock-{name}.mkv")),
                &out,
                AudioDecodeTransform {
                    channels: Some(2),
                    interval,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, (end - begin) as u64);
            let actual = pcm(&out);
            assert_eq!(actual.len(), (end - begin) * 2);
            for (a, b) in actual.iter().zip(&expected[begin * 2..end * 2]) {
                assert!((a - b).abs() < 2e-6, "{name}: {a}/{b}");
            }
        }
    }
    let out = dir.path().join("overlap.wav");
    let error = fvid_media::decode_audio_transformed(
        &root().join("aac-clock-jumps.mkv"),
        &out,
        AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("overlapping"), "{error}");
    assert!(!out.exists());
}

struct TestDir(PathBuf);
impl TestDir {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!("fvid-aac-{label}-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn public_aac_tail_padding_matches_spatial_oracle_and_bad_padding_cannot_publish() {
    let dir = TestDir::new("padding");
    let golden: Vec<f32> = std::fs::read(root().join("aac-device-layout/normal-ten.stereo.f32le"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let out = dir.path().join("tail.wav");
    let transform = fvid_media_info::AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    let stats = fvid_media::decode_audio_transformed(
        &root().join("aac-clock-tail.mkv"),
        &out,
        transform,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 6131);
    let actual = pcm(&out);
    assert_eq!(actual.len(), 6131 * 2);
    for (a, b) in actual.iter().zip(&golden) {
        assert!((a - b).abs() < 2e-6);
    }
    let out = dir.path().join("bad-tail.wav");
    let error = fvid_media::decode_audio_transformed(
        &root().join("aac-clock-invalid-tail.mkv"),
        &out,
        transform,
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("discard padding exceeds audio packet"),
        "{error}"
    );
    assert!(!out.exists());
}
