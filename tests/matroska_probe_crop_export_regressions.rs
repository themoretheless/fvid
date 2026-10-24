use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-probe-crop-export")
}

#[test]
fn native_matroska_probe_preserves_pcm_alac_mp1_codecs_custom_tags_writing_app_and_track_dispositions() {
    let source = root().join("matroska-probe-parity.mkv");
    let info = fvid::native_probe::probe(&source).unwrap();

    // 1. File-level metadata includes standard tags, custom SimpleTags, and WritingApp encoder fallback:
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Probe Parity Clip")
    );
    assert_eq!(
        info.metadata.get("CUSTOM_FILE_KEY").map(String::as_str),
        Some("custom-file-val")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("FVidTestMuxer 1.0")
    );

    // 2. Audio codecs are canonicalized across PCM bit depths, ALAC, and MPEG Layer I:
    assert_eq!(info.streams.len(), 4);
    assert_eq!(info.streams[0].codec, "pcm_s16le");
    assert_eq!(info.streams[1].codec, "pcm_f32le");
    assert_eq!(info.streams[2].codec, "alac");
    assert_eq!(info.streams[3].codec, "mp1");

    // 3. Track dispositions and track-scoped metadata / LanguageIETF / default language are preserved:
    // Track 0: FlagDefault=0, FlagForced=1 (64), LanguageIETF="en-US", TargetTrackUID=0 + TargetTrackUID=101
    assert_eq!(info.streams[0].disposition, 64);
    assert_eq!(
        info.streams[0].metadata.get("title").map(String::as_str),
        Some("Primary PCM")
    );
    assert_eq!(
        info.streams[0].metadata.get("language").map(String::as_str),
        Some("en-US")
    );
    assert_eq!(
        info.streams[0]
            .metadata
            .get("GLOBAL_TRACK_TAG")
            .map(String::as_str),
        Some("all-tracks")
    );
    assert_eq!(
        info.streams[0]
            .metadata
            .get("TRACK_ROLE")
            .map(String::as_str),
        Some("dialogue-stem")
    );

    // Track 1: FlagDefault=1 (1) | FlagCommentary=1 (8) = 9, default language "eng", global track tag only
    assert_eq!(info.streams[1].disposition, 9);
    assert_eq!(
        info.streams[1].metadata.get("language").map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        info.streams[1]
            .metadata
            .get("GLOBAL_TRACK_TAG")
            .map(String::as_str),
        Some("all-tracks")
    );
    assert!(!info.streams[1].metadata.contains_key("TRACK_ROLE"));
}

#[test]
fn identity_video_decode_and_y4m_export_apply_matroska_pixel_crop_insets_and_visible_aspect() {
    let source = root().join("matroska-cropped-16x16-to-12x8.mkv");

    // 1. Identity decode_video must apply container PixelCrop* (left=2, right=2, top=4, bottom=4 on 16x16 -> 12x8),
    // matching decode_video_transformed with a flip or filter:
    let identity_stats = fvid::native_media::decode_video(&source).unwrap();
    let flipped_stats = fvid::native_media::decode_video_transformed(
        &source,
        None,
        &fvid::native_geometry::VideoGeometry {
            horizontal_flip: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((identity_stats.width, identity_stats.height), (12, 8));
    assert_eq!(
        (identity_stats.width, identity_stats.height),
        (flipped_stats.width, flipped_stats.height)
    );
    assert_eq!(identity_stats.video_frames, 2);

    // 2. Identity Y4M export must also crop to 12x8 and preserve the container pixel aspect (2:1):
    let y4m_identity = std::env::temp_dir().join(format!(
        "fvid-batch-199-identity-{}.y4m",
        std::process::id()
    ));
    let _ = fs::remove_file(&y4m_identity);
    let packets = fvid::native_export::export_y4m(&source, &y4m_identity).unwrap();
    assert_eq!(packets, 2);
    let y4m_bytes = fs::read(&y4m_identity).unwrap();
    fs::remove_file(&y4m_identity).unwrap();
    let header_line = y4m_bytes
        .split(|&b| b == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .unwrap();
    assert!(
        header_line.contains("W12 H8") && header_line.contains("A2:1"),
        "unexpected identity Y4M header: {header_line}"
    );
    // 12x8 4:2:0 8-bit frame = 12*8 + 2*(6*4) = 144 bytes per frame + 6 bytes ("FRAME\n") = 150 bytes * 2 = 300 bytes after header
    assert_eq!(y4m_bytes.len() - (header_line.len() + 1), 300);

    // 3. Scaled Y4M export (scaling visible 12x8 with SAR 2:1 -> DAR 24:8 = 3:1 into 24x8 square-pixel target)
    // must compute transformed_aspect from visible dimensions (12x8), yielding A1:1 (not A2:3 from uncropped 16x16):
    let y4m_scaled = std::env::temp_dir().join(format!(
        "fvid-batch-199-scaled-{}.y4m",
        std::process::id()
    ));
    let _ = fs::remove_file(&y4m_scaled);
    let scale_geom = fvid::native_geometry::VideoGeometry {
        scale: Some([24, 8]),
        ..Default::default()
    };
    fvid::native_export::export_y4m_transformed(&source, &y4m_scaled, None, &scale_geom).unwrap();
    let scaled_bytes = fs::read(&y4m_scaled).unwrap();
    fs::remove_file(&y4m_scaled).unwrap();
    let scaled_header = scaled_bytes
        .split(|&b| b == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .unwrap();
    assert!(
        scaled_header.contains("W24 H8") && scaled_header.contains("A1:1"),
        "unexpected scaled Y4M header: {scaled_header}"
    );
}

#[test]
fn audio_source_info_and_decode_audio_plan_report_canonical_pcm_codec_names() {
    // 1. Matroska 16-bit little-endian PCM:
    let mka = root().join("matroska-pcm-s16le.mka");
    let mka_info = fvid::native_media::audio_source_info_selected(&mka, None).unwrap();
    assert_eq!(mka_info.codec, "pcm_s16le");
    let mka_plan = fvid::native_plan::decode_audio(&mka, &Default::default()).unwrap();
    assert_eq!(mka_plan.streams[0].codec, "pcm_s16le");

    // 2. Multi-track Matroska with explicit selection of stream 0 (pcm_s16le) and stream 1 (pcm_f32le):
    let multi = root().join("matroska-probe-parity.mkv");
    let s0 = fvid::native_media::audio_source_info_selected(&multi, Some(0)).unwrap();
    let s1 = fvid::native_media::audio_source_info_selected(&multi, Some(1)).unwrap();
    assert_eq!(s0.codec, "pcm_s16le");
    assert_eq!(s1.codec, "pcm_f32le");

    // 3. MP4 sowt 16-bit little-endian PCM:
    let mp4_sowt = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-selection/disabled-first-sowt.mp4");
    let mp4_info = fvid::native_media::audio_source_info_selected(&mp4_sowt, None).unwrap();
    assert_eq!(mp4_info.codec, "pcm_s16le");
    let mp4_plan = fvid::native_plan::decode_audio(&mp4_sowt, &Default::default()).unwrap();
    assert_eq!(mp4_plan.streams[0].codec, "pcm_s16le");
}
