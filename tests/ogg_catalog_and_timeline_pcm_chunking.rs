use fvid_control::CopyOptions;
use fvid_media::owned_ogg::{Limits as OggLimits, Ogg};
use fvid_media::{decode_audio_transformed, plan_decode_audio};
use fvid_media_info::AudioDecodeTransform;
use std::path::Path;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch598-{}-{}-{}",
        label,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ogg_audio_catalog_and_ogg_parse_fallible_packet_assembly_succeed() {
    let ogg_path = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-stereo.ogg");
    let bytes = std::fs::read(ogg_path).expect("read ogg fixture");
    let ogg = Ogg::parse(&bytes, &OggLimits::default()).expect("parse ogg");
    assert_eq!(ogg.chains().len(), 1);
    let tracks = fvid_media::owned_ogg_audio_catalog::open(
        std::io::Cursor::new(&bytes),
        OggLimits::default(),
    )
    .expect("open ogg audio catalog");
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].audio.channels, 2);
}

#[test]
fn matroska_and_mp4_audio_timeline_chunked_pcm_and_single_pass_descriptor_succeed() {
    let dir = unique_temp_dir("timeline-pcm");
    let mka_src = Path::new("tests/fixtures/playback-errors/container-loudness-plan/flac-loudness.mka");
    let mka_out = dir.join("mka-out.wav");
    let mka_plan = plan_decode_audio(
        mka_src,
        &AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("plan mka audio decode");
    assert_eq!(mka_plan.streams.len(), 1);
    let mka_stats = decode_audio_transformed(
        mka_src,
        &mka_out,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("decode mka audio");
    assert!(mka_stats.sample_frames > 0);

    let mp4_src = Path::new("tests/fixtures/playback-errors/container-loudness-plan/aac-loudness.m4a");
    if mp4_src.exists() {
        let mp4_out = dir.join("mp4-out.wav");
        let mp4_plan = plan_decode_audio(
            mp4_src,
            &AudioDecodeTransform::default(),
            &CopyOptions::default(),
        )
        .expect("plan mp4 audio decode");
        assert_eq!(mp4_plan.streams.len(), 1);
        let mp4_stats = decode_audio_transformed(
            mp4_src,
            &mp4_out,
            AudioDecodeTransform::default(),
            &CopyOptions::default(),
        )
        .expect("decode mp4 audio");
        assert!(mp4_stats.sample_frames > 0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
