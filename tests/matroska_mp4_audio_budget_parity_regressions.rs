//! Regression tests for Batch 257 (`FVID-0264`):
//! Multitrack Matroska/MP4 Audio Export, Loudnorm, Mix/Merge, and Metadata Extraction
//! Selected-Track Packet-Budget Parity.

use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-mp4-audio-budget-parity")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch257-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn multitrack_matroska_audio_export_and_plan_ignore_unselected_video_packet_sizes() {
    let dir = fixture_dir();
    let aac_mkv = dir.join("multitrack-vp9-aac.mkv");
    let pcm_mkv = dir.join("multitrack-vp9-pcm.mkv");
    let scratch = temp_dir("mkv-export");

    // Track 0 is VP9 video (1024-byte packets), track 1 is AAC (~28-byte packets) or PCM (16-byte packets).
    // With max_packet_bytes = 128, unselected video track 0 must not cause a false packet-budget failure.
    let mut options = fvid_media::CopyOptions::default();
    options.streams = vec![1];
    options.max_packet_bytes = 128;

    // 1. Top-level native_plan::decode_audio_with_options and fvid_media::plan_decode_audio
    let plan = fvid::native_plan::decode_audio_with_options(
        &aac_mkv,
        &Default::default(),
        &options,
    )
    .expect("native_plan::decode_audio_with_options should admit 128-byte audio packet budget");
    assert_eq!(plan.streams[0].index, 1);

    let media_plan = fvid_media::plan_decode_audio(&aac_mkv, &Default::default(), &options)
        .expect("fvid_media::plan_decode_audio should admit 128-byte audio packet budget");
    assert_eq!(media_plan.streams[0].index, 1);

    let pcm_plan = fvid_media::plan_decode_audio(&pcm_mkv, &Default::default(), &options)
        .expect("fvid_media::plan_decode_audio on PCM MKV should admit 128-byte audio packet budget");
    assert_eq!(pcm_plan.streams[0].index, 1);

    // 2. Top-level export_audio_pcm_transformed_with_controls to .wav and .mka
    let out_wav = scratch.join("aac.wav");
    let stats_wav = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &aac_mkv,
        &out_wav,
        None,
        1.0,
        None,
        None,
        &options,
    )
    .expect("export to .wav should succeed with 128-byte packet budget");
    assert_eq!(stats_wav.sample_frames, 2048);

    let out_mka = scratch.join("aac.mka");
    let stats_mka = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &aac_mkv,
        &out_mka,
        None,
        1.0,
        None,
        None,
        &options,
    )
    .expect("export to .mka should succeed and preserve metadata with 128-byte packet budget");
    assert_eq!(stats_mka.sample_frames, 2048);
    let probe_mka = fvid::media::probe(&out_mka).expect("probe exported .mka");
    assert_eq!(
        probe_mka.streams[0].metadata.get("title").map(String::as_str),
        Some("Main Audio")
    );

    // 3. fvid_media::decode_audio on both AAC MKV and PCM MKV
    let media_aac_wav = scratch.join("media-aac.wav");
    let media_aac_stats = fvid_media::decode_audio(&aac_mkv, &media_aac_wav, &options)
        .expect("fvid_media::decode_audio on AAC MKV should succeed with 128-byte packet budget");
    assert_eq!(media_aac_stats.sample_frames, 2048);

    let media_pcm_wav = scratch.join("media-pcm.wav");
    let media_pcm_stats = fvid_media::decode_audio(&pcm_mkv, &media_pcm_wav, &options)
        .expect("fvid_media::decode_audio on PCM MKV should succeed with 128-byte packet budget");
    assert_eq!(media_pcm_stats.sample_frames, 16);

    // 4. Tight budget (4 bytes) smaller than the selected audio track packets must still fail!
    let mut tight = options.clone();
    tight.max_packet_bytes = 4;
    let err = fvid::native_plan::decode_audio_with_options(&aac_mkv, &Default::default(), &tight)
        .expect_err("4-byte budget must reject 6-byte AAC packets");
    assert!(err.contains("WebM packet exceeds budget"), "unexpected error: {err}");

    let err_pcm = fvid_media::decode_audio(&pcm_mkv, &scratch.join("tight-pcm.wav"), &tight)
        .expect_err("4-byte budget must reject 16-byte PCM packets");
    assert!(
        err_pcm.contains("WebM packet exceeds budget"),
        "unexpected error: {err_pcm}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn multitrack_matroska_loudnorm_and_loudness_ignore_unselected_video_packet_sizes() {
    let dir = fixture_dir();
    let aac_mkv = dir.join("multitrack-vp9-aac.mkv");
    let scratch = temp_dir("mkv-loudnorm");

    let mut options = fvid_media::CopyOptions::default();
    options.streams = vec![1];
    options.max_packet_bytes = 128;

    // 1. Loudness measurement & planning in fvid_media
    let loud_plan = fvid_media::plan_loudness(&aac_mkv, &options)
        .expect("fvid_media::plan_loudness should admit 128-byte audio packet budget");
    assert_eq!(loud_plan.streams[0].index, 1);
    let loud_stats = fvid_media::measure_loudness(&aac_mkv, &options)
        .expect("fvid_media::measure_loudness should succeed with 128-byte audio packet budget");
    assert_eq!(loud_stats.sample_frames, 2048);

    // 2. Top-level native_loudnorm::try_plan & try_apply to .wav and .mka
    let norm_plan = fvid::native_loudnorm::try_plan(&aac_mkv, None, false, &options)
        .expect("native_loudnorm::try_plan should not error on unselected video track")
        .expect("native_loudnorm::try_plan should return Some(plan)");
    assert_eq!(norm_plan.streams[0].index, 1);

    let out_wav = scratch.join("norm.wav");
    let norm_wav_stats = fvid::native_loudnorm::try_apply(&aac_mkv, &out_wav, None, false, &options)
        .expect("native_loudnorm::try_apply to .wav should not error on unselected video track")
        .expect("native_loudnorm::try_apply to .wav should return Some(stats)");
    assert!(norm_wav_stats.sample_frames > 0);

    let out_mka = scratch.join("norm.mka");
    let norm_mka_stats = fvid::native_loudnorm::try_apply(&aac_mkv, &out_mka, None, false, &options)
        .expect("native_loudnorm::try_apply to .mka should not error on unselected video track")
        .expect("native_loudnorm::try_apply to .mka should return Some(stats)");
    assert!(norm_mka_stats.sample_frames > 0);

    // 3. Tight budget (4 bytes) must still fail on the selected audio track
    let mut tight = options.clone();
    tight.max_packet_bytes = 4;
    let err = fvid::native_loudnorm::try_plan(&aac_mkv, None, false, &tight)
        .expect_err("4-byte budget must reject 6-byte AAC packets in loudnorm");
    assert!(
        err.to_string().contains("WebM packet exceeds budget"),
        "unexpected error: {err}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn multitrack_matroska_and_mp4_mix_and_merge_audio_ignore_video_packet_sizes() {
    let dir = fixture_dir();
    let mkv_a = dir.join("multitrack-vp9-aac.mkv");
    let mkv_b = dir.join("multitrack-vp9-aac-alt.mkv");
    let mp4_a = dir.join("multitrack-avc-aac.mp4");
    let scratch = temp_dir("mix-merge");

    // Both MKV (1024-byte VP9 video, 6-byte AAC audio) and MP4 (~778-byte AVC video, 6-byte AAC audio)
    // have exactly 1 audio stream (stream 1), so they are valid inputs for mix-audio and merge-audio.
    // With max_packet_bytes = 128, video packets exceed 128 bytes while audio packets are <= 64 bytes.
    let mut controls = fvid_media::CopyOptions::default();
    controls.max_packet_bytes = 128;

    let mix_inputs = vec![mkv_a.clone(), mkv_b.clone()];
    assert!(
        fvid::native_audio_mix::eligible_with_options(&mix_inputs, &controls)
            .expect("eligible_with_options on MKV inputs"),
        "MKV inputs with 128-byte budget should be eligible for mix-audio"
    );
    let mix_opts = fvid::native_audio_mix::MixAudioOptions::default();
    fvid::native_audio_mix::plan_with_controls(&mix_inputs, &mix_opts, &controls)
        .expect("mix-audio plan_with_controls on MKV inputs should succeed");

    let mixed_mka = scratch.join("mixed.mka");
    let mix_stats = fvid::native_audio_mix::mix_audio_with_controls(
        &mix_inputs,
        &mixed_mka,
        &mix_opts,
        &controls,
    )
    .expect("mix_audio_with_controls to .mka on MKV inputs should succeed");
    assert_eq!(mix_stats.sample_frames, 2048);

    let mixed_wav = scratch.join("mixed.wav");
    let mix_wav_stats = fvid::native_audio_mix::mix_audio_with_controls(
        &mix_inputs,
        &mixed_wav,
        &mix_opts,
        &controls,
    )
    .expect("mix_audio_with_controls to .wav on MKV inputs should succeed");
    assert_eq!(mix_wav_stats.sample_frames, 2048);

    // MP4 + MKV merge-audio with 128-byte packet budget
    let merge_inputs = vec![mp4_a.clone(), mkv_a.clone()];
    assert!(
        fvid::native_audio_mix::eligible_with_options(&merge_inputs, &controls)
            .expect("eligible_with_options on MP4 + MKV inputs"),
        "MP4 + MKV inputs with 128-byte budget should be eligible"
    );
    fvid::native_audio_mix::plan_merge_with_controls(&merge_inputs, &controls)
        .expect("plan_merge_with_controls on MP4 + MKV inputs should succeed");

    let merged_mka = scratch.join("merged.mka");
    let merge_stats = fvid::native_audio_mix::merge_audio_with_controls(
        &merge_inputs,
        &merged_mka,
        &controls,
    )
    .expect("merge_audio_with_controls to .mka on MP4 + MKV inputs should succeed");
    assert_eq!(merge_stats.sample_frames, 2048);
    assert_eq!(merge_stats.channels, 2);

    // Tight budget (4 bytes) smaller than the AAC audio packets must reject eligibility and plan!
    let mut tight = controls.clone();
    tight.max_packet_bytes = 4;
    assert!(
        !fvid::native_audio_mix::eligible_with_options(&mix_inputs, &tight)
            .expect("eligible_with_options with 4-byte budget"),
        "4-byte budget must not be eligible for 6-byte AAC packets"
    );
    assert!(
        !fvid::native_audio_mix::eligible_with_options(&vec![mp4_a.clone(), mp4_a.clone()], &tight)
            .expect("eligible_with_options on MP4 with 4-byte budget"),
        "4-byte budget on MP4 must not be eligible for 6-byte AAC packets"
    );
    let err = fvid::native_audio_mix::plan_with_controls(&mix_inputs, &mix_opts, &tight)
        .expect_err("4-byte budget must fail mix-audio plan");
    assert!(err.to_string().contains("WebM packet exceeds budget"), "unexpected error: {err}");

    let _ = std::fs::remove_dir_all(&scratch);
}
