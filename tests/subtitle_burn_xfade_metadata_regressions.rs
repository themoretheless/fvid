use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn-xfade-metadata")
}

#[test]
fn subtitle_burn_single_track_mp4_gray_rgb_and_set_then_delete_metadata_precedence() {
    let dir = fixture_dir();
    let mp4_src = dir.join("avc-single-rotated-burn.mp4");
    let srt_src = dir.join("cues.srt");
    let y4m_burn = dir.join("burn-source.y4m");
    let xfade_main = dir.join("xfade-main.y4m");
    let xfade_other = dir.join("xfade-other.y4m");

    let temp = std::env::temp_dir().join(format!(
        "fvid-subtitle-burn-xfade-metadata-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Shared set-then-delete metadata options:
    // - Container: set TITLE="DeletedTitle", CUSTOM_DEL="Gone", ARTIST="KeptArtist", CUSTOM_KEEP="KeptCustom"
    //   and delete TITLE, custom_del.
    // - Stream 0: set TITLE="DeletedStreamTitle", LANGUAGE="fra", TRACK_DEL="GoneTrack", TRACK_KEEP="KeptTrack"
    //   and delete TITLE, language, track_del.
    let mut meta_opts = fvid::media_control::CopyOptions::default();
    meta_opts.metadata_set = vec![
        ("title".into(), "DeletedTitle".into()),
        ("custom_del".into(), "Gone".into()),
        ("artist".into(), "KeptArtist".into()),
        ("custom_keep".into(), "KeptCustom".into()),
    ];
    meta_opts.metadata_delete = vec!["TITLE".into(), "CUSTOM_DEL".into()];
    meta_opts.stream_metadata_set = vec![
        (0, "title".into(), "DeletedStreamTitle".into()),
        (0, "language".into(), "fra".into()),
        (0, "track_del".into(), "GoneTrack".into()),
        (0, "track_keep".into(), "KeptTrack".into()),
    ];
    meta_opts.stream_metadata_delete = vec![
        (0, "TITLE".into()),
        (0, "LANGUAGE".into()),
        (0, "TRACK_DEL".into()),
    ];

    // 1. Single-track rotated MP4 subtitle burn-in (`supports`, `plan_burn_subtitles`, `burn_subtitles`):
    assert!(
        fvid_media::owned_subtitle_burn::supports(&mp4_src, &srt_src, &meta_opts),
        "owned_subtitle_burn::supports must admit single-track MP4 video"
    );
    let mp4_burn_plan = fvid_media::plan_burn_subtitles(&mp4_src, &srt_src, &meta_opts)
        .expect("plan_burn_subtitles must support single-track rotated MP4");
    assert_eq!(mp4_burn_plan.command, "burn-subtitles");
    assert_eq!(mp4_burn_plan.streams.len(), 1);
    assert!(mp4_burn_plan.graph.is_none());

    let out_mp4_burn = temp.join("mp4-burned.mkv");
    let mp4_burn_stats =
        fvid_media::burn_subtitles(&mp4_src, &out_mp4_burn, &srt_src, &meta_opts)
            .expect("burn_subtitles must support single-track rotated MP4");
    assert_eq!(mp4_burn_stats.backend, "owned SRT burn-in FFV1 export");
    assert_eq!(mp4_burn_stats.video_frames, 2);
    assert_eq!(mp4_burn_stats.decoded_frames, 2);
    let mut mp4_burn_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_mp4_burn).unwrap()),
        Default::default(),
    )
    .unwrap();
    mp4_burn_webm.scan_all().unwrap();
    assert_eq!(mp4_burn_webm.tracks.len(), 1);
    // 64x32 coded rotated by 90 deg -> 32x64 upright display
    assert_eq!(mp4_burn_webm.tracks[0].width, 32);
    assert_eq!(mp4_burn_webm.tracks[0].height, 64);
    assert_eq!(mp4_burn_webm.tracks[0].rotation, 0);
    assert_eq!(mp4_burn_webm.tags.title, "");
    assert_eq!(mp4_burn_webm.tags.artist, "KeptArtist");
    assert!(!mp4_burn_webm.metadata.contains_key("CUSTOM_DEL"));
    assert_eq!(
        mp4_burn_webm.metadata.get("CUSTOM_KEEP").map(String::as_str),
        Some("KeptCustom")
    );
    assert_eq!(mp4_burn_webm.tracks[0].name, "");
    assert_eq!(mp4_burn_webm.tracks[0].language, "");
    assert_eq!(
        mp4_burn_webm
            .track_languages
            .get(&mp4_burn_webm.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let uid0 = mp4_burn_webm.track_uids[&mp4_burn_webm.tracks[0].number];
    let track_tags0 = mp4_burn_webm.track_metadata.get(&uid0).unwrap();
    assert!(!track_tags0.contains_key("TRACK_DEL"));
    assert_eq!(
        track_tags0.get("TRACK_KEEP").map(String::as_str),
        Some("KeptTrack")
    );
    // Verify frame 0 (t=0 ms) has burned white text pixels (luma > 32), while frame 1 (t=500 ms) is untouched (luma == 32):
    let mut ffv1_mp4 = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        32,
        64,
        &mp4_burn_webm.tracks[0].codec_private,
        32 << 20,
    )
    .unwrap();
    let f0 = ffv1_mp4.decode(&mp4_burn_webm.read_packet(0).unwrap()).unwrap();
    let f1 = ffv1_mp4.decode(&mp4_burn_webm.read_packet(1).unwrap()).unwrap();
    assert!(
        f0.frame.data[..32 * 64].iter().any(|&y| y > 32),
        "frame 0 must contain burned subtitle pixels"
    );
    assert!(
        f1.frame.data[..32 * 64].iter().all(|&y| y == 32),
        "frame 1 outside cue interval must remain untouched"
    );

    // 2. Y4M subtitle burn-in set-then-delete metadata precedence:
    let out_y4m_burn = temp.join("y4m-burned.mkv");
    fvid_media::burn_subtitles(&y4m_burn, &out_y4m_burn, &srt_src, &meta_opts).unwrap();
    let mut y4m_burn_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_y4m_burn).unwrap()),
        Default::default(),
    )
    .unwrap();
    y4m_burn_webm.scan_all().unwrap();
    assert_eq!(y4m_burn_webm.tags.title, "");
    assert_eq!(y4m_burn_webm.tags.artist, "KeptArtist");
    assert!(!y4m_burn_webm.metadata.contains_key("CUSTOM_DEL"));
    assert_eq!(
        y4m_burn_webm.metadata.get("CUSTOM_KEEP").map(String::as_str),
        Some("KeptCustom")
    );
    assert_eq!(y4m_burn_webm.tracks[0].name, "");
    assert_eq!(y4m_burn_webm.tracks[0].language, "");
    assert_eq!(
        y4m_burn_webm
            .track_languages
            .get(&y4m_burn_webm.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let uid_y4m = y4m_burn_webm.track_uids[&y4m_burn_webm.tracks[0].number];
    let track_tags_y4m = y4m_burn_webm.track_metadata.get(&uid_y4m).unwrap();
    assert!(!track_tags_y4m.contains_key("TRACK_DEL"));
    assert_eq!(
        track_tags_y4m.get("TRACK_KEEP").map(String::as_str),
        Some("KeptTrack")
    );

    // 3. Y4M cross-fade (`owned_xfade::xfade_video`) set-then-delete metadata precedence:
    let out_xfade = temp.join("xfade-out.mkv");
    fvid_media::xfade_video(
        &xfade_main,
        &xfade_other,
        &out_xfade,
        "fade",
        500_000,
        0,
        &meta_opts,
    )
    .unwrap();
    let mut xfade_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_xfade).unwrap()),
        Default::default(),
    )
    .unwrap();
    xfade_webm.scan_all().unwrap();
    assert_eq!(xfade_webm.tags.title, "");
    assert_eq!(xfade_webm.tags.artist, "KeptArtist");
    assert!(!xfade_webm.metadata.contains_key("CUSTOM_DEL"));
    assert_eq!(
        xfade_webm.metadata.get("CUSTOM_KEEP").map(String::as_str),
        Some("KeptCustom")
    );
    assert_eq!(xfade_webm.tracks[0].name, "");
    assert_eq!(xfade_webm.tracks[0].language, "");
    assert_eq!(
        xfade_webm
            .track_languages
            .get(&xfade_webm.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let uid_xf = xfade_webm.track_uids[&xfade_webm.tracks[0].number];
    let track_tags_xf = xfade_webm.track_metadata.get(&uid_xf).unwrap();
    assert!(!track_tags_xf.contains_key("TRACK_DEL"));
    assert_eq!(
        track_tags_xf.get("TRACK_KEEP").map(String::as_str),
        Some("KeptTrack")
    );

    // 4. Y4M lossless export (`owned_lossless::transcode_lossless` -> `write_y4m_ffv1_policy`)
    // set-then-delete metadata precedence:
    let out_lossless = temp.join("y4m-lossless.mkv");
    fvid_media::transcode_lossless(
        &xfade_main,
        &out_lossless,
        Default::default(),
        &meta_opts,
    )
    .unwrap();
    let mut lossless_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_lossless).unwrap()),
        Default::default(),
    )
    .unwrap();
    lossless_webm.scan_all().unwrap();
    assert_eq!(lossless_webm.tags.title, "");
    assert_eq!(lossless_webm.tags.artist, "KeptArtist");
    assert!(!lossless_webm.metadata.contains_key("CUSTOM_DEL"));
    assert_eq!(
        lossless_webm.metadata.get("CUSTOM_KEEP").map(String::as_str),
        Some("KeptCustom")
    );
    assert_eq!(lossless_webm.tracks[0].name, "");
    assert_eq!(lossless_webm.tracks[0].language, "");
    assert_eq!(
        lossless_webm
            .track_languages
            .get(&lossless_webm.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let uid_ll = lossless_webm.track_uids[&lossless_webm.tracks[0].number];
    let track_tags_ll = lossless_webm.track_metadata.get(&uid_ll).unwrap();
    assert!(!track_tags_ll.contains_key("TRACK_DEL"));
    assert_eq!(
        track_tags_ll.get("TRACK_KEEP").map(String::as_str),
        Some("KeptTrack")
    );

    // 5. Subtitle conversion (`convert_subtitles_with_controls`) set-then-delete metadata precedence
    // and uppercase custom tag normalization:
    let out_subs = temp.join("subs-converted.mkv");
    fvid::media::convert_subtitles_with_controls(
        &srt_src,
        &out_subs,
        &fvid::media_info::SubtitleConvertOptions {
            streams: Vec::new(),
            codec: fvid::media_info::SubtitleCodec::Ass,
        },
        &meta_opts,
    )
    .unwrap();
    let mut subs_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_subs).unwrap()),
        Default::default(),
    )
    .unwrap();
    subs_webm.scan_all().unwrap();
    assert_eq!(subs_webm.tags.title, "");
    assert_eq!(subs_webm.tags.artist, "KeptArtist");
    assert!(!subs_webm.metadata.contains_key("CUSTOM_DEL"));
    assert!(!subs_webm.metadata.contains_key("custom_del"));
    assert_eq!(
        subs_webm.metadata.get("CUSTOM_KEEP").map(String::as_str),
        Some("KeptCustom")
    );
    assert_eq!(subs_webm.tracks[0].name, "");
    assert_eq!(subs_webm.tracks[0].language, "");
    assert_eq!(
        subs_webm
            .track_languages
            .get(&subs_webm.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let uid_sub = subs_webm.track_uids[&subs_webm.tracks[0].number];
    let track_tags_sub = subs_webm.track_metadata.get(&uid_sub).unwrap();
    assert!(!track_tags_sub.contains_key("TRACK_DEL"));
    assert!(!track_tags_sub.contains_key("track_del"));
    assert_eq!(
        track_tags_sub.get("TRACK_KEEP").map(String::as_str),
        Some("KeptTrack")
    );

    // 6. Monochrome (`V_FFV1` gray) and RGB subtitle burn-in:
    // Build a 64x32 10-bit monochrome FFV1 Matroska file and an 8-bit RGB FFV1 Matroska file,
    // burn subtitles onto both, and verify output remains monochrome (`has_chroma == false`) or RGB!
    let gray_mkv = temp.join("gray10-source.mkv");
    {
        let mut file = File::create(&gray_mkv).unwrap();
        let mut writer = fvid_media::owned_matroska::PacketWriter::new_ffv1(&mut file, 64, 32).unwrap();
        let gray_samples = 40u16.to_le_bytes().repeat(64 * 32);
        let pkt = fvid_media::owned_ffv1_encoder::encode_gray(64, 32, &gray_samples, 10).unwrap();
        writer.write_packet(0, 0, 500_000_000, true, &pkt).unwrap();
        writer.finish().unwrap();
    }
    let out_gray_burn = temp.join("gray10-burned.mkv");
    let gray_burn_stats = fvid_media::burn_subtitles(
        &gray_mkv,
        &out_gray_burn,
        &srt_src,
        &Default::default(),
    )
    .expect("burn_subtitles must support 10-bit monochrome FFV1 Matroska");
    assert_eq!(gray_burn_stats.pixel_format, "gray10le");
    let mut gray_burn_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_gray_burn).unwrap()),
        Default::default(),
    )
    .unwrap();
    gray_burn_webm.scan_all().unwrap();
    let mut gray_dec = fvid_media::owned_ffv1_decoder::Decoder::new(64, 32, 32 << 20).unwrap();
    let gray_f0 = gray_dec.decode(&gray_burn_webm.read_packet(0).unwrap()).unwrap();
    assert_eq!(
        gray_dec.monochrome(),
        Some(true),
        "burned monochrome FFV1 stream must remain true monochrome without chroma planes"
    );
    assert!(
        gray_f0.frame.data[..64 * 32 * 2]
            .chunks_exact(2)
            .any(|b| u16::from_le_bytes([b[0], b[1]]) > 40),
        "burned monochrome FFV1 frame must contain white text luma samples"
    );

    let rgb_mkv = temp.join("rgb8-source.mkv");
    {
        let mut file = File::create(&rgb_mkv).unwrap();
        let mut writer = fvid_media::owned_matroska::PacketWriter::new_ffv1(&mut file, 64, 32).unwrap();
        let rgb_frame = fvid_media::owned_frame::GeometryFrame {
            width: 64,
            height: 32,
            subsampling: None,
            data: [10u8, 20, 30].repeat(64 * 32),
        };
        let pkt = fvid_media::owned_ffv1_encoder::encode(&rgb_frame, 8).unwrap();
        writer.write_packet(0, 0, 500_000_000, true, &pkt).unwrap();
        writer.finish().unwrap();
    }
    let out_rgb_burn = temp.join("rgb8-burned.mkv");
    let rgb_burn_stats = fvid_media::burn_subtitles(
        &rgb_mkv,
        &out_rgb_burn,
        &srt_src,
        &Default::default(),
    )
    .expect("burn_subtitles must support RGB FFV1 Matroska");
    assert_eq!(rgb_burn_stats.pixel_format, "rgb24");
    let mut rgb_burn_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_rgb_burn).unwrap()),
        Default::default(),
    )
    .unwrap();
    rgb_burn_webm.scan_all().unwrap();
    let mut rgb_dec = fvid_media::owned_ffv1_decoder::Decoder::new(64, 32, 32 << 20).unwrap();
    let rgb_f0 = rgb_dec.decode(&rgb_burn_webm.read_packet(0).unwrap()).unwrap();
    assert_eq!(rgb_f0.frame.subsampling, None);
    assert!(
        rgb_f0.frame.data.chunks_exact(3).any(|px| px[0] > 10 && px[1] > 20 && px[2] > 30),
        "burned RGB FFV1 frame must contain white text RGB samples"
    );

    let _ = std::fs::remove_dir_all(&temp);
}
