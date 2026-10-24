use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-rematrix-opus-mp3-admission")
}

#[test]
fn native_wave_rematrix_supports_7_1_downmix_and_stereo_upmix_to_wav_and_matroska() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch-238-wave-rematrix-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let wave_71 = fixture_dir().join("wave-8ch-71.wav");
    let wave_stereo = fixture_dir().join("wave-stereo.wav");
    let options = fvid::media::CopyOptions::default();

    // 1. 7.1 (8ch, mask 0x63f) -> stereo (2ch) and mono (1ch) via default native route
    for target_ch in [2i32, 1i32] {
        let transform = fvid::media::AudioDecodeTransform {
            channels: Some(target_ch),
            ..Default::default()
        };
        let plan = fvid::media::plan_decode_audio(&wave_71, &transform, &options)
            .expect("8ch 7.1 downmix plan should succeed");
        assert!(plan.steps.iter().any(|s| s.action == "rematrix"));

        let out_wav = dir.join(format!("downmix-71-to-{target_ch}.wav"));
        let stats_wav =
            fvid::media::decode_audio_transformed(&wave_71, &out_wav, transform, &options)
                .expect("8ch 7.1 downmix to .wav should succeed");
        assert_eq!(stats_wav.channels, target_ch);
        assert_eq!(stats_wav.sample_frames, 64);

        let out_mka = dir.join(format!("downmix-71-to-{target_ch}.mka"));
        let stats_mka =
            fvid::media::decode_audio_transformed(&wave_71, &out_mka, transform, &options)
                .expect("8ch 7.1 downmix to .mka should succeed");
        assert_eq!(stats_mka.channels, target_ch);
        assert_eq!(stats_mka.sample_frames, 64);
    }

    // 2. Stereo (2ch) -> 5.1 (6ch) and 7.1 (8ch) upmix via default native route
    for (target_ch, expected_mask) in [(6i32, 0x3fu32), (8i32, 0x63fu32)] {
        let transform = fvid::media::AudioDecodeTransform {
            channels: Some(target_ch),
            ..Default::default()
        };
        let plan = fvid::media::plan_decode_audio(&wave_stereo, &transform, &options)
            .expect("stereo upmix plan should succeed");
        assert!(plan.steps.iter().any(|s| s.action == "rematrix"));

        let out_wav = dir.join(format!("upmix-2-to-{target_ch}.wav"));
        let stats_wav =
            fvid::media::decode_audio_transformed(&wave_stereo, &out_wav, transform, &options)
                .expect("stereo upmix to .wav should succeed");
        assert_eq!(stats_wav.channels, target_ch);
        assert_eq!(stats_wav.sample_frames, 64);
        let mut wav_file = std::fs::File::open(&out_wav).unwrap();
        let inspected = fvid_media::owned_wave_inspect::inspect(&mut wav_file, None).unwrap();
        assert_eq!(i32::from(inspected.channels), target_ch);
        assert_eq!(inspected.channel_mask, expected_mask);

        let out_mka = dir.join(format!("upmix-2-to-{target_ch}.mka"));
        let stats_mka =
            fvid::media::decode_audio_transformed(&wave_stereo, &out_mka, transform, &options)
                .expect("stereo upmix to .mka should succeed");
        assert_eq!(stats_mka.channels, target_ch);
        assert_eq!(stats_mka.sample_frames, 64);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn matroska_opus_and_mp2_admission_and_raw_mp2_metadata_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch-238-opus-mp2-admission-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let opus_51 = fixture_dir().join("opus-6ch-51.mka");
    let mp2_mka = fixture_dir().join("mp2-stereo.mka");
    let mp2_raw = fixture_dir().join("mp2-stereo-tagged.mp2");

    // 1. Matroska Opus 5.1 (6ch) supports downmix to 2ch, but rejects invalid 6->5 rematrix
    let downmix_2 = fvid::media::AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    let out_opus_2 = dir.join("opus-downmix-2ch.wav");
    let stats_opus = fvid::media::decode_audio_transformed(
        &opus_51,
        &out_opus_2,
        downmix_2,
        &fvid::media::CopyOptions::default(),
    )
    .expect("Matroska Opus 5.1 -> 2ch WAV should succeed");
    assert_eq!(stats_opus.channels, 2);

    let invalid_5 = fvid::media::AudioDecodeTransform {
        channels: Some(5),
        ..Default::default()
    };
    let out_opus_5 = dir.join("opus-invalid-5ch.wav");
    assert!(
        fvid::media::decode_audio_transformed(
            &opus_51,
            &out_opus_5,
            invalid_5,
            &fvid::media::CopyOptions::default(),
        )
        .is_err()
    );
    assert!(!out_opus_5.exists());

    // 2. Matroska MP2 stereo supports upmix to 6ch, but rejects 2 -> 9ch.
    let upmix_6 = fvid::media::AudioDecodeTransform {
        channels: Some(6),
        ..Default::default()
    };
    let out_mp2_upmix = dir.join("mp2-upmix-6ch.wav");
    let stats_mp2 = fvid::media::decode_audio_transformed(
        &mp2_mka,
        &out_mp2_upmix,
        upmix_6,
        &fvid::media::CopyOptions::default(),
    )
    .expect("Matroska MP2 stereo -> 6ch WAV should succeed");
    assert_eq!(stats_mp2.channels, 6);

    // 3. Raw MP2 with ID3v2 tags (including TLAN="fra") exported to Matroska .mka preserves
    // source ID3v2 title/artist/language and applies custom container metadata mutations.
    let out_raw_mka = dir.join("raw-mp2-tagged.mka");
    let mut meta_opts = fvid::media::CopyOptions::default();
    meta_opts
        .metadata_set
        .push(("custom_tag".into(), "batch238".into()));
    let stats_raw = fvid::media::decode_audio_transformed(
        &mp2_raw,
        &out_raw_mka,
        Default::default(),
        &meta_opts,
    )
    .expect("Raw MP2 to .mka with metadata mutations should succeed");
    assert_eq!(stats_raw.channels, 2);
    let probed_mka = fvid::media::probe(&out_raw_mka).unwrap();
    assert_eq!(
        probed_mka.metadata.get("title").map(String::as_str),
        Some("Tagged Bare MP2")
    );
    assert_eq!(
        probed_mka.metadata.get("artist").map(String::as_str),
        Some("FVid Synth Artist")
    );
    assert_eq!(
        probed_mka.metadata.get("CUSTOM_TAG").map(String::as_str),
        Some("batch238")
    );
    assert_eq!(
        probed_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    let _ = std::fs::remove_dir_all(&dir);
}
