use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-prefix-budget-mp4-wave-info")
        .join(name)
}

fn temp_wav_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fvid-batch262-{}-{}-{}.wav",
        tag,
        std::process::id(),
        nanos
    ))
}

#[test]
fn matroska_flac_and_pcm_packet_byte_budget_scopes_to_max_packets_prefix() {
    let flac_mka = fixture("prefix-flac.mka");
    let pcm_mka = fixture("prefix-pcm.mka");

    // 1. Matroska A_FLAC: packet 0 is 11 bytes (192 frames), packet 1 is 393 bytes.
    // With max_packet_bytes = 50 and max_packets = Some(1), packet 0 is within budget and
    // packet 1 is outside the active prefix, so plan/decode/loudnorm/mix/merge must succeed.
    let flac_prefix_opts = CopyOptions {
        max_packet_bytes: 50,
        max_packets: Some(1),
        ..CopyOptions::default()
    };
    let flac_plan = media::plan_decode_audio(
        &flac_mka,
        &AudioDecodeTransform::default(),
        &flac_prefix_opts,
    )
    .expect("plan_decode_audio on prefix-bounded Matroska A_FLAC should succeed");
    assert_eq!(flac_plan.streams.len(), 1);
    assert_eq!(flac_plan.streams[0].codec, "flac");

    let flac_wav = temp_wav_path("flac-prefix");
    let flac_stats = media::decode_audio_transformed(
        &flac_mka,
        &flac_wav,
        AudioDecodeTransform::default(),
        &flac_prefix_opts,
    )
    .expect("decode_audio_transformed on prefix-bounded Matroska A_FLAC should succeed");
    let _ = std::fs::remove_file(&flac_wav);
    assert_eq!(flac_stats.decoded_frames, 1);
    assert_eq!(flac_stats.sample_frames, 192);

    let flac_ln_plan = media::plan_loudnorm(&flac_mka, None, false, &flac_prefix_opts)
        .expect("plan_loudnorm on prefix-bounded Matroska A_FLAC should succeed");
    assert!(!flac_ln_plan.steps.is_empty());

    let flac_ln_wav = temp_wav_path("flac-prefix-ln");
    let flac_ln_stats = media::apply_loudnorm(
        &flac_mka,
        &flac_ln_wav,
        None,
        &flac_prefix_opts,
    )
    .expect("apply_loudnorm on prefix-bounded Matroska A_FLAC should succeed");
    let _ = std::fs::remove_file(&flac_ln_wav);
    assert_eq!(flac_ln_stats.sample_rate, 192_000);
    assert_eq!(flac_ln_stats.sample_frames, 768);

    let flac_mix_wav = temp_wav_path("flac-prefix-mix");
    let flac_mix_inputs = vec![flac_mka.clone(), flac_mka.clone()];
    let flac_mix_stats = media::mix_audio_with_controls(
        &flac_mix_inputs,
        &flac_mix_wav,
        &MixAudioOptions::default(),
        &flac_prefix_opts,
    )
    .expect("mix_audio_with_controls on prefix-bounded Matroska A_FLAC should succeed");
    let _ = std::fs::remove_file(&flac_mix_wav);
    assert_eq!(flac_mix_stats.sample_frames, 192);

    let flac_merge_wav = temp_wav_path("flac-prefix-merge");
    let flac_merge_stats = media::merge_audio_with_controls(
        &flac_mix_inputs,
        &flac_merge_wav,
        &flac_prefix_opts,
    )
    .expect("merge_audio_with_controls on prefix-bounded Matroska A_FLAC should succeed");
    let _ = std::fs::remove_file(&flac_merge_wav);
    assert_eq!(flac_merge_stats.channels, 2);
    assert_eq!(flac_merge_stats.sample_frames, 192);

    // When max_packets is Some(2), packet 1 (393B > 50B) is inside the prefix and must be rejected.
    let flac_two_opts = CopyOptions {
        max_packet_bytes: 50,
        max_packets: Some(2),
        ..CopyOptions::default()
    };
    let flac_err = media::plan_decode_audio(
        &flac_mka,
        &AudioDecodeTransform::default(),
        &flac_two_opts,
    )
    .expect_err("2-packet prefix must reject 393-byte packet #2 under 50-byte budget")
    .to_string();
    assert!(
        flac_err.contains("packet exceeds budget"),
        "unexpected error: {flac_err}"
    );

    // 2. Matroska A_PCM/INT/LIT: packet 0 is 16 bytes (8 frames), packet 1 is 128 bytes.
    let pcm_prefix_opts = CopyOptions {
        max_packet_bytes: 32,
        max_packets: Some(1),
        ..CopyOptions::default()
    };
    let pcm_plan = media::plan_decode_audio(
        &pcm_mka,
        &AudioDecodeTransform::default(),
        &pcm_prefix_opts,
    )
    .expect("plan_decode_audio on prefix-bounded Matroska A_PCM should succeed");
    assert_eq!(pcm_plan.streams.len(), 1);
    assert_eq!(pcm_plan.streams[0].codec, "pcm_s16le");

    let pcm_controlled_opts = CopyOptions {
        max_packet_bytes: 32,
        max_packets: Some(1),
        max_controlled_bytes: Some(2 * 1024 * 1024),
        ..CopyOptions::default()
    };
    let pcm_controlled_plan = media::plan_decode_audio(
        &pcm_mka,
        &AudioDecodeTransform::default(),
        &pcm_controlled_opts,
    )
    .expect("controlled plan_decode_audio on prefix-bounded Matroska A_PCM should succeed");
    assert_eq!(pcm_controlled_plan.streams.len(), 1);

    let pcm_wav = temp_wav_path("pcm-prefix");
    let pcm_stats = media::decode_audio_transformed(
        &pcm_mka,
        &pcm_wav,
        AudioDecodeTransform::default(),
        &pcm_prefix_opts,
    )
    .expect("decode_audio_transformed on prefix-bounded Matroska A_PCM should succeed");
    let _ = std::fs::remove_file(&pcm_wav);
    assert_eq!(pcm_stats.decoded_frames, 1);
    assert_eq!(pcm_stats.sample_frames, 8);

    let pcm_two_opts = CopyOptions {
        max_packet_bytes: 32,
        max_packets: Some(2),
        ..CopyOptions::default()
    };
    let pcm_err = media::plan_decode_audio(
        &pcm_mka,
        &AudioDecodeTransform::default(),
        &pcm_two_opts,
    )
    .expect_err("2-packet prefix must reject 128-byte packet #2 under 32-byte budget")
    .to_string();
    assert!(
        pcm_err.contains("packet exceeds budget"),
        "unexpected error: {pcm_err}"
    );
}

#[test]
fn mp4_aac_prefix_budget_and_wave_export_preserve_itunes_encoder_tag() {
    let mp4_path = fixture("tagged-encoder.m4a");

    let probed_src = media::probe(&mp4_path).expect("probe tagged-encoder.m4a");
    assert_eq!(
        probed_src.metadata.get("title").map(String::as_str),
        Some("MP4 Tagged Track")
    );
    assert_eq!(
        probed_src.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-muxer 1.0")
    );

    let wav_export = temp_wav_path("mp4-export");
    media::decode_audio_transformed(
        &mp4_path,
        &wav_export,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("decode_audio_transformed MP4 -> WAV");
    let probed_export = media::probe(&wav_export).expect("probe exported WAV");
    let _ = std::fs::remove_file(&wav_export);
    assert_eq!(
        probed_export.metadata.get("title").map(String::as_str),
        Some("MP4 Tagged Track")
    );
    assert_eq!(
        probed_export.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-muxer 1.0"),
        "MP4 ©too encoder tag must be preserved as RIFF INFO ISFT on WAV export"
    );

    let wav_owned = temp_wav_path("mp4-owned-export");
    fvid_media::decode_audio_transformed(
        &mp4_path,
        &wav_owned,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("fvid_media::decode_audio_transformed MP4 -> WAV");
    let probed_owned = media::probe(&wav_owned).expect("probe owned exported WAV");
    let _ = std::fs::remove_file(&wav_owned);
    assert_eq!(
        probed_owned.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-muxer 1.0")
    );

    let wav_ln = temp_wav_path("mp4-loudnorm");
    media::apply_loudnorm(
        &mp4_path,
        &wav_ln,
        None,
        &CopyOptions::default(),
    )
    .expect("apply_loudnorm MP4 -> WAV");
    let probed_ln = media::probe(&wav_ln).expect("probe loudnorm WAV");
    let _ = std::fs::remove_file(&wav_ln);
    assert_eq!(
        probed_ln.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-muxer 1.0"),
        "MP4 ©too encoder tag must be preserved as RIFF INFO ISFT on loudnorm WAV output"
    );
}
