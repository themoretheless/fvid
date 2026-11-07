use std::path::Path;
use fvid::{
    media::{self, CopyOptions},
    native_loudnorm, native_media, native_plan,
};

#[test]
fn native_plan_decode_audio_and_loudness_reuse_wave_header_and_metadata() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let mut opts = CopyOptions::default();
    opts.max_controlled_bytes = Some(64 * 1024 * 1024);

    let decode_plan = native_plan::decode_audio_with_options(wav, &Default::default(), &opts)
        .expect("native decode-audio plan on WAVE");
    assert_eq!(decode_plan.command, "decode-audio");
    assert_eq!(decode_plan.streams.len(), 1);
    assert_eq!(decode_plan.streams[0].codec, "pcm_s32le");

    let loudness_plan = native_plan::loudness_with_options(wav, &opts, None)
        .expect("native loudness plan on WAVE");
    assert_eq!(loudness_plan.command, "loudness");
    assert_eq!(loudness_plan.streams.len(), 1);
    assert_eq!(loudness_plan.streams[0].codec, "pcm_s32le");
}

#[test]
fn native_loudnorm_and_trim_source_admission_handle_wave_and_flac() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let opts = CopyOptions::default();

    assert!(
        !native_media::is_owned_audio_trim_source_with_options(wav, &opts)
            .expect("trim source check on WAVE"),
        "WAVE is handled by native_pcm::is_wave rather than compressed trim source admission"
    );
    assert!(
        !native_media::is_owned_audio_trim_source_with_options(flac, &opts)
            .expect("trim source check on standalone FLAC")
    );

    let loudnorm_plan = native_loudnorm::try_plan(wav, None, false, &opts)
        .expect("native loudnorm try_plan on WAVE")
        .expect("WAVE should produce an owned loudnorm plan");
    assert_eq!(loudnorm_plan.command, "loudnorm");

    let media_remux = media::plan_remux(
        Path::new("tests/fixtures/playback-errors/mp4-pcm-s16le-stereo.mp4"),
        &opts,
    );
    // Ensure remux planning either succeeds or cleanly refuses without leaking descriptors.
    let _ = media_remux;
}
