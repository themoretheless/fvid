use std::path::PathBuf;

use fvid_control::CopyOptions;
use fvid_media::{decode_audio_transformed, plan_decode_audio, probe};
use fvid_media_info::AudioDecodeTransform;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("audio-plan-mp3-flac-and-matroska-pcm")
}

#[test]
fn standalone_packet_work_counts_leading_mp3_info_packet_and_active_presentation_window() {
    let mp3 = fixture_dir().join("mp3-info-prefix-interval.mp3");
    let options = CopyOptions::default();

    // The fixture has 1 leading LAME Info packet and 4 coded frames (4608 samples),
    // with end_padding = 1800 (tail = 1800 - 529 = 1271 trimmed samples ->
    // presentation_window = 529..3337), so 3 audio frames (pts 0, 1152, 2304) are active
    // and frame 3 (pts 3456 >= 3337) is past presentation_window.end.
    // Total packet work during decode is 1 (Info) + 3 (active frames) = 4 packets.
    let work = fvid_media::owned_audio_plan::standalone_packet_work(&mp3, &options)
        .expect("standalone_packet_work should succeed")
        .expect("bare MP3 should report standalone packet work");
    assert_eq!(work, 4);

    let mut limited = CopyOptions::default();
    limited.max_packets = Some(2);
    let work_limited = fvid_media::owned_audio_plan::standalone_packet_work(&mp3, &limited)
        .expect("standalone_packet_work with max_packets should succeed")
        .expect("bare MP3 should report bounded standalone packet work");
    assert_eq!(work_limited, 2);
}

#[test]
fn plan_decode_audio_rejects_bare_flac_interval_past_max_packets_prefix() {
    let flac = fixture_dir().join("flac-two-frame-prefix-interval.flac");
    // Frame 0 covers [0, 192) samples = [0, 4000) us at 48 kHz;
    // Frame 1 covers [192, 384) samples = [4000, 8000) us.
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((5_000, 7_000));

    let mut limited = CopyOptions::default();
    limited.max_packets = Some(1);

    let err = plan_decode_audio(&flac, &transform, &limited)
        .expect_err("interval starting in frame 1 must fail when max_packets=1");
    assert!(
        err.to_string().contains("no decoded audio samples"),
        "unexpected error: {err}"
    );

    // With max_packets=2, frame 1 is decoded and the interval [5000, 7000) us is non-empty.
    limited.max_packets = Some(2);
    let plan = plan_decode_audio(&flac, &transform, &limited)
        .expect("interval within 2-packet prefix should succeed");
    assert!(!plan.steps.is_empty());
}

#[test]
fn plan_decode_audio_rejects_bare_mp3_interval_past_max_packets_prefix_and_info_only_limit() {
    let mp3 = fixture_dir().join("mp3-info-prefix-interval.mp3");

    // Packet 1 is the LAME Info packet, so max_packets=1 decodes 0 audio frames.
    let mut info_only = CopyOptions::default();
    info_only.max_packets = Some(1);
    let err_info = plan_decode_audio(
        &mp3,
        &AudioDecodeTransform::default(),
        &info_only,
    )
    .expect_err("max_packets=1 consumed by Info packet must be rejected during planning");
    assert!(
        err_info
            .to_string()
            .contains("MP3 packet limit contains no samples"),
        "unexpected error: {err_info}"
    );

    // With max_packets=2 (1 Info + 1 audio frame = 1152 samples ~= 26_122 us at 44.1 kHz),
    // an interval at [30_000, 40_000) us falls in audio frame 1 (packet 3) and must fail.
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((30_000, 40_000));
    let mut two_packets = CopyOptions::default();
    two_packets.max_packets = Some(2);
    let err_interval = plan_decode_audio(&mp3, &transform, &two_packets)
        .expect_err("interval past 2-packet MP3 prefix must be rejected");
    assert!(
        err_interval.to_string().contains("no decoded audio samples"),
        "unexpected error: {err_interval}"
    );

    // With max_packets=3 (1 Info + 2 audio frames = 2304 samples ~= 52_244 us), the interval succeeds.
    two_packets.max_packets = Some(3);
    let plan = plan_decode_audio(&mp3, &transform, &two_packets)
        .expect("interval within 3-packet MP3 prefix should succeed");
    assert!(!plan.steps.is_empty());
}

#[test]
fn plan_decode_audio_rejects_empty_interval_on_matroska_pcm() {
    let mka = fixture_dir().join("mka-pcm-language-ietf.mka");
    // Stream has 1920 samples at 48 kHz = 40_000 us.
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((50_000, 60_000));

    let err = plan_decode_audio(&mka, &transform, &CopyOptions::default())
        .expect_err("out-of-range interval on Matroska PCM must be rejected during planning");
    assert!(
        err.to_string().contains("no decoded audio samples"),
        "unexpected error: {err}"
    );
}

#[test]
fn matroska_pcm_wave_export_preserves_language_ietf_in_riff_info_ilng() {
    let mka = fixture_dir().join("mka-pcm-language-ietf.mka");
    let out_wav = std::env::temp_dir().join(format!(
        "fvid-mka-pcm-lang-ietf-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav);

    decode_audio_transformed(
        &mka,
        &out_wav,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("decode Matroska PCM to WAVE should succeed");

    let probed = probe(&out_wav).expect("probe exported WAVE");
    let _ = std::fs::remove_file(&out_wav);
    assert_eq!(
        probed.metadata.get("language").map(String::as_str),
        Some("pt-BR"),
        "exported WAVE should preserve Matroska LanguageIETF in ILNG/language metadata: {:?}",
        probed.metadata
    );
}
