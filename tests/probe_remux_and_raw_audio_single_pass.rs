use fvid_control::CopyOptions;
use fvid_media_info::AudioDecodeTransform;
use std::path::{Path, PathBuf};

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fvid-{label}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn wave_and_mp4_probe_succeed_with_fallible_reservations() {
    let mp4_path = fixtures_root().join("playback-errors/pcm32-precision-edits.mov");
    let mp4_info = fvid_media::owned_probe::probe(&mp4_path).expect("probe mp4");
    assert_eq!(mp4_info.streams.len(), 1);
    assert_eq!(mp4_info.streams[0].codec, "pcm_s32le");

    let dir = unique_temp_dir("wave-probe-fallible");
    let wav_path = dir.join("tagged.wav");
    fvid_media::owned_wav_file::write_wav_integer_le(&wav_path, 48000, 1, 16, &[1, 2, 3, 4], 4)
        .expect("write wav");
    let out_wav = dir.join("tagged-out.wav");
    fvid_media::remux(
        &wav_path,
        &out_wav,
        &CopyOptions {
            metadata_set: vec![("title".into(), "fallible info".into())],
            ..Default::default()
        },
    )
    .expect("remux wav with title");
    let wav_info = fvid_media::owned_probe::probe_wave(&out_wav).expect("probe wave");
    assert_eq!(
        wav_info.metadata.get("title").map(String::as_str),
        Some("fallible info")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn matroska_remux_concat_and_wave_remux_reject_existing_destination_upfront() {
    let dir = unique_temp_dir("remux-concat-dest-exists");
    let mkv_source = fixtures_root().join("playback-errors/webm-probe-clear.mkv");
    let existing_mkv = dir.join("existing.mkv");
    std::fs::write(&existing_mkv, b"sentinel").expect("write mkv sentinel");

    // Even when CopyOptions has streams=[99] (which is not an identity remux),
    // owned_matroska_remux::remux must reject the existing destination first.
    let err = fvid_media::owned_matroska_remux::remux(
        &mkv_source,
        &existing_mkv,
        &CopyOptions {
            streams: vec![99],
            ..Default::default()
        },
    )
    .expect_err("must reject existing destination first");
    assert!(
        err.contains("output already exists"),
        "expected 'output already exists', got: {err}"
    );
    assert_eq!(std::fs::read(&existing_mkv).expect("read sentinel"), b"sentinel");

    // owned_concat::concat must also reject existing destination before checking supports().
    let err = fvid_media::concat(
        &[mkv_source.clone()],
        &existing_mkv,
        &CopyOptions::default(),
    )
    .expect_err("concat must reject existing destination first");
    assert!(
        err.contains("output already exists"),
        "expected 'output already exists', got: {err}"
    );

    // owned_wave_remux::remux / trim / concat must reject existing .wav destination before policy checks.
    let wav_source = dir.join("src.wav");
    fvid_media::owned_wav_file::write_wav_integer_le(&wav_source, 48000, 1, 16, &[0; 8], 4)
        .expect("write src wav");
    let existing_wav = dir.join("existing.wav");
    std::fs::write(&existing_wav, b"wav-sentinel").expect("write wav sentinel");

    let bad_opts = CopyOptions {
        streams: vec![99],
        ..Default::default()
    };
    for res in [
        fvid_media::owned_wave_remux::remux(&wav_source, &existing_wav, &bad_opts),
        fvid_media::owned_wave_remux::trim(&wav_source, &existing_wav, 10, 5, &bad_opts),
        fvid_media::owned_wave_remux::concat(&[wav_source.clone()], &existing_wav, &bad_opts),
    ] {
        let err = res.expect_err("wave op must reject existing destination first");
        assert!(
            err.contains("output already exists"),
            "expected 'output already exists', got: {err}"
        );
    }
    assert_eq!(
        std::fs::read(&existing_wav).expect("read wav sentinel"),
        b"wav-sentinel"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raw_flac_and_mp3_wave_exports_reuse_parsed_source_without_rereading() {
    let dir = unique_temp_dir("flac-mp3-single-pass-decode");
    let flac_source =
        fixtures_root().join("playback-errors/audio-budget-controls/raw-flac-48k-440ms.flac");
    let flac_out = dir.join("flac.wav");
    let flac_stats = fvid_media::decode_audio_transformed(
        &flac_source,
        &flac_out,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("export flac to wav");
    assert!(flac_stats.sample_frames > 0);
    assert!(flac_out.exists());

    let mp3_source =
        fixtures_root().join("playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let mp3_out = dir.join("mp3.wav");
    let mp3_stats = fvid_media::decode_audio_transformed(
        &mp3_source,
        &mp3_out,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .expect("export mp3 to wav");
    assert!(mp3_stats.sample_frames > 0);
    assert!(mp3_out.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
