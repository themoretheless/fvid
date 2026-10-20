use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
#[test]
fn bare_flac_public_export_has_an_owned_plan() {
    let source = root().join("flac-native/16-1-verbatim-0.flac");
    let plan =
        fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default()).unwrap();
    assert_eq!(plan.streams[0].codec, "flac");
}
#[test]
fn bare_flac_full_and_interval_pcm_match_all_authored_integer_oracles() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("flac-matroska/manifest.json")).unwrap())
            .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let bits = case["bits"].as_u64().unwrap();
        let source = root().join(format!("flac-native/{name}.flac"));
        let probe = fvid_media::probe(&source).unwrap();
        assert_eq!(probe.streams[0].duration, Some(64), "{name}");
        assert_eq!(probe.streams[0].sample_rate, 8000);
        let integers = std::fs::read(root().join(format!("flac-native/{name}.s32le"))).unwrap();
        let expected: Vec<u8> = integers
            .chunks_exact(4)
            .flat_map(|b| {
                ((i32::from_le_bytes(b.try_into().unwrap()) as f64 / (1u64 << (bits - 1)) as f64)
                    as f32)
                    .to_le_bytes()
            })
            .collect();
        for interval in [
            None,
            Some((Duration::from_millis(2), Duration::from_millis(6))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_raw_flac::decode_raw_flac_pcm(
                &source,
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
fn public_bare_flac_exports_all_channel_counts_with_plans_dsp_and_controls() {
    let root = root();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("flac-matroska/manifest.json")).unwrap())
            .unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-raw-flac-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for channels in 1..=8u64 {
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["channels"].as_u64() == Some(channels) && c["bits"].as_u64() == Some(32))
            .unwrap();
        let name = case["name"].as_str().unwrap();
        let source = root.join(format!("flac-native/{name}.flac"));
        let plan = fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default())
            .unwrap();
        assert_eq!(plan.streams[0].codec, "flac");
        for hint in [None, Some("flac")] {
            let probe = fvid_media::probe_as(&source, hint).unwrap();
            assert_eq!(probe.format, "flac");
            assert_eq!(probe.streams[0].codec, "flac");
            assert_eq!(probe.streams[0].channels, channels as i32);
            assert_eq!(probe.streams[0].duration, Some(64));
            assert_eq!(probe.streams[0].time_base, [1, 8000]);
        }
        let full = dir.join(format!("{channels}-full.wav"));
        let stats = fvid_media::decode_audio(&source, &full, &Default::default()).unwrap();
        assert_eq!(stats.sample_frames, 64);
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&full).unwrap(), None)
                .unwrap();
        assert_eq!(
            info.channel_mask,
            match channels {
                1 => 4,
                2 => 3,
                3 => 7,
                4 => 0x33,
                5 => 0x37,
                6 => 0x3f,
                7 => 0x70f,
                _ => 0x63f,
            }
        );
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((2000, 6000)),
            sample_rate: Some(16000),
            channels: Some(1),
            volume: Some(0.5),
        };
        fvid_media::plan_decode_audio(&source, &transform, &Default::default()).unwrap();
        let actual = dir.join(format!("{channels}-actual.wav"));
        let reference = dir.join(format!("{channels}-reference.wav"));
        fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default())
            .unwrap();
        fvid_media::decode_audio_transformed(&full, &reference, transform, &Default::default())
            .unwrap();
        assert_eq!(
            std::fs::read(actual).unwrap(),
            std::fs::read(reference).unwrap()
        );
    }
    let source = root.join("flac-native/16-1-verbatim-0.flac");
    for (label, options, reason) in [
        (
            "memory",
            fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
        (
            "packet",
            fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            "FLAC packet bytes exceed limit",
        ),
        (
            "stream",
            fvid_control::CopyOptions {
                streams: vec![1],
                ..Default::default()
            },
            "stream 0",
        ),
    ] {
        let out = dir.join(format!("{label}.wav"));
        let e = fvid_media::decode_audio(&source, &out, &options).unwrap_err();
        assert!(e.contains(reason), "{label}: {e}");
        assert!(!out.exists());
    }
    let flag = fvid_control::CancelFlag::new();
    flag.cancel();
    let out = dir.join("cancelled.wav");
    assert!(fvid_media::decode_audio(
        &source,
        &out,
        &fvid_control::CopyOptions {
            cancel: Some(flag),
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(!out.exists());
    let mut pcm = Vec::new();
    fvid_media::owned_raw_flac::decode_raw_flac_pcm(
        &source,
        &mut pcm,
        None,
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(64 << 20),
            max_packets: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pcm.len(), 64 * 2 * 4);
}
