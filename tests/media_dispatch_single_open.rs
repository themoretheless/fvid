use fvid::media::{
    self, AudioDecodeTransform, CopyOptions, EncoderSettings, LosslessTransform,
};
use std::path::Path;

#[test]
fn media_plan_decode_audio_and_remux_use_single_prefix_pass() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let options = CopyOptions::default();

    let audio_plan = media::plan_decode_audio(
        wav,
        &AudioDecodeTransform::default(),
        &options,
    )
    .expect("WAVE decode_audio plan should succeed");
    assert_eq!(audio_plan.command, "decode-audio");

    let concat_plan = media::plan_concat(&[wav.to_path_buf(), wav.to_path_buf()], &options)
        .expect("WAVE concat plan should succeed");
    assert_eq!(concat_plan.command, "concat");

    let mut zero_pkt_options = CopyOptions::default();
    zero_pkt_options.max_packets = Some(0);
    let err = media::plan_remux(
        Path::new("tests/fixtures/playback-errors/avc-sps-32x16.mp4"),
        &zero_pkt_options,
    )
    .expect_err("zero max_packets MP4 plan_remux should fail");
    assert!(err.contains("requires at least one packet"));
}

#[test]
fn media_transcode_validates_lossless_eligibility_once() {
    let source = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let mut options = CopyOptions::default();
    options.max_packets = Some(0);
    let err = media::transcode(
        source,
        Path::new("unused.mkv"),
        LosslessTransform::default(),
        &options,
        &EncoderSettings {
            name: "ffv1".into(),
            options: Vec::new(),
        },
    )
    .expect_err("zero max_packets transcode should be rejected");
    assert!(err.contains("requires at least one packet"));
}
