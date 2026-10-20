use std::{fs, io::Cursor, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/player-controls")
}
fn text(name: &str) -> String {
    fs::read_to_string(root().join(name)).unwrap()
}
fn control() {
    assert_eq!(
        fvid_media::owned_y4m_decode::decode_video(&root().join("control.y4m"))
            .unwrap()
            .video_frames,
        3
    );
}

#[test]
fn malformed_and_overflowing_subtitle_clocks_do_not_panic_or_change_valid_cues() {
    control();
    let cues = fvid_media::parse_srt(&text("overflow-clock.srt"));
    assert_eq!(cues.len(), 1);
    assert_eq!(
        (cues[0].start_us, cues[0].end_us, cues[0].text.as_str()),
        (0, 40_000, "Valid")
    );
    for bad in [
        "9223372036854775807:00:00,000",
        "00:60:00,000",
        "00:00:60,000",
        "-1:00:00,000",
        "00:00:00.1junk",
        "00:00:00.\u{0661}",
        "00:00:00.",
    ] {
        assert!(fvid_media::parse_subtitle_clock(bad).is_none(), "{bad}");
        assert!(
            fvid_media::parse_webvtt_timestamp(&bad.replace(',', ".")).is_none(),
            "VTT {bad}"
        );
    }
    for (good, expected) in [
        ("0:00:01.20", 1_200_000),
        ("01:02", 62_000_000),
        ("90:00.001", 5_400_001_000),
        ("00:00:00.123456", 123_000),
    ] {
        assert_eq!(fvid_media::parse_subtitle_clock(good), Some(expected));
    }
    let millis = i64::MAX / 1000;
    let boundary = format!(
        "{}:{:02}:{:02}.{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1000 % 60,
        millis % 1000
    );
    assert_eq!(
        fvid_media::parse_subtitle_clock(&boundary),
        Some(millis * 1000)
    );
}

#[test]
fn nonfinite_and_overflowing_clip_times_are_refused() {
    control();
    assert!(fvid_media::parse_smil_clip_line(&text("invalid-time.smil")).is_none());
    assert!(fvid_media::parse_edl_line(&text("invalid-time.edl")).is_none());
    for value in ["NaN", "inf", "-inf", "1e100", "-1e100"] {
        assert!(fvid_media::parse_smil_clip_line(&format!("control.y4m@{value}")).is_none());
        assert!(
            fvid_media::parse_smil_clip_line(&format!("src=control.y4m begin={value}")).is_none()
        );
        assert!(fvid_media::parse_edl_line(&format!("control.y4m 0 {value}")).is_none());
        assert!(fvid_media::parse_edl_line(&format!("control.y4m in=0 out={value}")).is_none());
    }
    assert!(fvid_media::parse_smil_clip_line("@1").is_none());
    assert!(fvid_media::parse_smil_clip_line("src=\"\" begin=1").is_none());
    assert!(fvid_media::parse_edl_line("control.y4m -2 -1").is_none());
    // Preserve the existing useful clamp for a range crossing the start.
    let clip = fvid_media::parse_edl_line("control.y4m -1 1").unwrap();
    assert_eq!((clip.in_us, clip.out_us), (0, 1_000_000));
    assert_eq!(
        fvid_media::parse_smil_clip_line("control.y4m@-1")
            .unwrap()
            .begin_us,
        -1_000_000
    );
}

#[test]
fn unicode_metadata_is_safe_at_osd_byte_cutoffs() {
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(fs::read(root().join("unicode-osd.webm")).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 1);
    let decoded = fvid_codecs::codec::vp9_decoder::Decoder::new(32 << 20)
        .decode(&reader.read_packet(0).unwrap())
        .unwrap();
    assert_eq!(decoded.picture.size, [64, 64]);
    let art = reader.metadata.get("CHAPTER_ART_URL").unwrap();
    let fingerprint = reader.metadata.get("MEDIA_FINGERPRINT").unwrap();
    assert_eq!(
        fvid_media::format_chapter_art_osd(Some(art)),
        format!("Chapter art {}…", "a".repeat(47))
    );
    assert_eq!(
        fvid_media::format_media_fingerprint_osd(fingerprint),
        format!("Fingerprint {}…", "a".repeat(11))
    );
    assert_eq!(
        fvid_media::format_media_fingerprint_osd("abcdef0123456789"),
        "Fingerprint abcdef012345…"
    );
}

#[test]
fn quoted_and_at_sign_clip_paths_remain_openable() {
    let smil = fvid_media::parse_smil_clip_line(&text("quoted-path.smil")).unwrap();
    assert_eq!(
        (smil.src.as_str(), smil.begin_us),
        ("clip with spaces.y4m", 40_000)
    );
    let edl = fvid_media::parse_edl_line(&text("quoted-path.edl")).unwrap();
    assert_eq!(
        (edl.src.as_str(), edl.in_us, edl.out_us),
        ("clip with spaces.y4m", 40_000, 80_000)
    );
    let at = fvid_media::parse_smil_clip_line("clip@name.y4m@0.04").unwrap();
    assert_eq!((at.src.as_str(), at.begin_us), ("clip@name.y4m", 40_000));
    let keyed = fvid_media::parse_smil_clip_line("src=clip@name.y4m begin=0.04").unwrap();
    assert_eq!(keyed, at);
    for path in [smil.src, edl.src, at.src] {
        assert_eq!(
            fvid_media::owned_y4m_decode::decode_video(&root().join(path))
                .unwrap()
                .video_frames,
            3
        );
    }
    assert!(fvid_media::parse_smil_clip_line("src=\"unterminated begin=1").is_none());
    assert!(fvid_media::parse_edl_line("\"unterminated 0 1").is_none());
}
