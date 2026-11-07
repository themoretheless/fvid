use fvid_control::CopyOptions;
use fvid_media::{decode_audio_transformed, plan_burn_subtitles, plan_decode_audio};
use fvid_media_info::AudioDecodeTransform;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

fn unique_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "fvid-ogg-g711-burn-flac-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ogg_vorbis_and_opus_single_pass_parse_and_fallible_reserve() {
    let dir = unique_temp_dir("ogg");
    let vorbis = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-stereo.ogg");
    let opus = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg");

    for (index, src) in [vorbis, opus].into_iter().enumerate() {
        let out = dir.join(format!("out-{index}.wav"));
        let plan = plan_decode_audio(src, &AudioDecodeTransform::default(), &CopyOptions::default())
            .expect("plan ogg audio");
        assert_eq!(plan.streams.len(), 1);
        let stats = decode_audio_transformed(
            src,
            &out,
            AudioDecodeTransform::default(),
            &CopyOptions::default(),
        )
        .expect("decode ogg audio");
        assert!(stats.sample_frames > 0);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn g711_wave_and_matroska_flac_single_pass_descriptor_and_subtitle_burn() {
    let dir = unique_temp_dir("g711-flac-burn");
    let g711 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/g711-alaw-48k-mono.wav");
    let g711_plan = plan_decode_audio(
        g711,
        &AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("plan g711 wave");
    assert_eq!(g711_plan.streams[0].codec, "pcm_alaw");

    let mka_flac = Path::new("tests/fixtures/playback-errors/container-loudness-plan/flac-loudness.mka");
    let flac_plan = plan_decode_audio(
        mka_flac,
        &AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("plan matroska flac");
    assert_eq!(flac_plan.streams[0].codec, "flac");
    let flac_out = dir.join("flac.wav");
    let flac_stats = decode_audio_transformed(
        mka_flac,
        &flac_out,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("decode matroska flac");
    assert!(flac_stats.sample_frames > 0);

    let y4m = Path::new("tests/fixtures/playback-errors/subtitle-burn.y4m");
    let srt = Path::new("tests/fixtures/playback-errors/subtitle-burn.srt");
    let burn_opts = CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..CopyOptions::default()
    };
    let burn_plan = plan_burn_subtitles(y4m, srt, &burn_opts).expect("plan subtitle burn");
    assert_eq!(burn_plan.command, "burn-subtitles");

    let _ = std::fs::remove_dir_all(&dir);
}
