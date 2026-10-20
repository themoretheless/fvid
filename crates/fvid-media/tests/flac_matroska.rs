use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
#[test]
fn owned_flac_matroska_full_and_interval_pcm_match_authored_integer_oracles() {
    let root = root();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("flac-matroska/manifest.json")).unwrap())
            .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let bits = case["bits"].as_u64().unwrap();
        let video = std::fs::read(root.join(format!("flac-matroska/{name}.mkv"))).unwrap();
        let integer = std::fs::read(root.join(format!("flac-native/{name}.s32le"))).unwrap();
        let expected: Vec<u8> = integer
            .chunks_exact(4)
            .flat_map(|v| {
                ((i32::from_le_bytes(v.try_into().unwrap()) as f64 / (1u64 << (bits - 1)) as f64)
                    as f32)
                    .to_le_bytes()
            })
            .collect();
        for interval in [
            None,
            Some((Duration::from_millis(2), Duration::from_millis(6))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
                Cursor::new(&video),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let wanted = if interval.is_some() {
                &expected[16 * channels * 4..48 * channels * 4]
            } else {
                &expected
            };
            assert_eq!(pcm, wanted, "{name}");
            assert_eq!(
                stats.sample_frames,
                wanted.len() as u64 / channels as u64 / 4
            );
        }
    }
}
#[test]
fn public_flac_export_plan_dsp_and_admission_use_owned_decoder() {
    let source = root().join("flac-matroska/16-1-verbatim-0.mkv");
    let dir = std::env::temp_dir().join(format!("fvid-flac-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let plan =
        fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default()).unwrap();
    assert_eq!(plan.streams[0].codec, "flac");
    let wave = dir.join("full.wav");
    let stats = fvid_media::decode_audio(&source, &wave, &Default::default()).unwrap();
    assert_eq!(stats.sample_frames, 64);
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((2000, 6000)),
        sample_rate: Some(16000),
        channels: Some(1),
        volume: Some(0.5),
    };
    let actual = dir.join("actual.wav");
    let reference = dir.join("reference.wav");
    fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default()).unwrap();
    fvid_media::decode_audio_transformed(&wave, &reference, transform, &Default::default())
        .unwrap();
    // Identical DSP and container writer must yield identical public outputs.
    assert_eq!(
        std::fs::read(actual).unwrap(),
        std::fs::read(reference).unwrap()
    );
    let limited = dir.join("limited.wav");
    let e = fvid_media::decode_audio(
        &source,
        &limited,
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(e.contains("controlled memory budget exceeded"), "{e}");
    assert!(!limited.exists());
    let mut pcm = Vec::new();
    fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
        Cursor::new(std::fs::read(&source).unwrap()),
        &mut pcm,
        None,
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(64 << 20),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pcm.len(), 64 * 2 * 4);
    let cancelled = dir.join("cancelled.wav");
    let flag = fvid_control::CancelFlag::new();
    flag.cancel();
    assert!(
        fvid_media::decode_audio(
            &source,
            &cancelled,
            &fvid_control::CopyOptions {
                cancel: Some(flag),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(!cancelled.exists());
}
#[test]
fn four_channel_flac_export_preserves_quad_speaker_mask() {
    let source = root().join("flac-matroska/extended-lpc-4-1-0-0.mkv");
    let dir = std::env::temp_dir().join(format!("fvid-flac-quad-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let out = dir.join("quad.wav");
    fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
    let info =
        fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&out).unwrap(), None)
            .unwrap();
    assert_eq!(info.channels, 4);
    assert_eq!(info.channel_mask, 0x33);
    let transform = fvid_media_info::AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    let actual = dir.join("actual.wav");
    let expected = dir.join("expected.wav");
    fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default()).unwrap();
    fvid_media::decode_audio_transformed(&out, &expected, transform, &Default::default()).unwrap();
    assert_eq!(
        std::fs::read(actual).unwrap(),
        std::fs::read(expected).unwrap()
    );
}

#[test]
fn positive_flac_container_origin_is_preserved_in_full_and_interval_exports() {
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("flac-matroska/manifest.json")).unwrap())
            .unwrap();
    for c in info["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let bits = c["bits"].as_u64().unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let raw = std::fs::read(root().join(format!("flac-native/{name}.s32le"))).unwrap();
        let mut expected = vec![0; (rate / 20) as usize * channels * 4];
        expected.extend(raw.chunks_exact(4).flat_map(|v| {
            ((i32::from_le_bytes(v.try_into().unwrap()) as f64 / (1u64 << (bits - 1)) as f64)
                as f32)
                .to_le_bytes()
        }));
        let movie = std::fs::read(root().join(format!("flac-matroska/{name}-offset.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(49), Duration::from_millis(52))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
                Cursor::new(&movie),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            let wanted = if interval.is_some() {
                &expected[(49 * rate).div_ceil(1000) as usize * channels * 4
                    ..(52 * rate).div_ceil(1000) as usize * channels * 4]
            } else {
                &expected
            };
            assert_eq!(pcm, wanted, "{name}");
            assert_eq!(
                stats.sample_frames,
                wanted.len() as u64 / channels as u64 / 4
            );
        }
    }
}

#[test]
fn multi_packet_flac_export_uses_coded_sample_clock() {
    let expected: Vec<u8> = [4096, -4096, 8192, -8192]
        .into_iter()
        .flat_map(|level| (0..64).flat_map(move |_| (level as f32 / 32768.).to_le_bytes()))
        .collect();
    for name in ["unit", "third", "tail", "discard-last"] {
        let tail = if name == "tail" {
            13
        } else if name == "discard-last" {
            64
        } else {
            0
        };
        let expected = &expected[..(256 - tail) * 4];
        let bytes = std::fs::read(root().join(format!("flac-timeline/{name}.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_micros(1500), Duration::from_micros(4000))),
        ] {
            let wanted = if interval.is_some() {
                &expected[(1500u64 * 44100).div_ceil(1_000_000) as usize * 4
                    ..(4000u64 * 44100).div_ceil(1_000_000) as usize * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(pcm, wanted);
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 4);
        }
    }
}
