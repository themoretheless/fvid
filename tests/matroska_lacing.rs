use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn laced_ffv1_packets_decode_with_exact_positions_and_frame_clocks() {
    for mode in ["xiph", "ebml", "fixed"] {
        let source = fixture(&format!("matroska-lace-{mode}.mkv"));
        let bytes = std::fs::read(&source).unwrap();
        let mut reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 4);
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 3, 1 << 20).unwrap();
        for i in 0..4 {
            let packet = reader.packets[i].clone();
            assert_eq!(packet.pts_ns, 5000000 + i as i64 * 40000000);
            assert_eq!(packet.duration_ns, Some(40000000));
            assert_eq!(packet.discard_padding_ns, 0);
            assert!(packet.keyframe);
            let index = if mode == "fixed" { 0 } else { i % 2 };
            let expected = std::fs::read(fixture(&format!("ffv1-gray-8-{index}.packet"))).unwrap();
            let actual = reader.read_packet(i).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(
                &bytes[packet.offset as usize..packet.offset as usize + packet.size],
                expected
            );
            let decoded = decoder.decode(&actual).unwrap();
            let luma = std::fs::read(fixture(&format!("ffv1-gray-8-{index}.gray"))).unwrap();
            assert_eq!(&decoded.frame.data[..luma.len()], luma);
        }
        for i in [3, 0, 2, 1] {
            let index = if mode == "fixed" { 0 } else { i % 2 };
            assert_eq!(
                reader.read_packet(i).unwrap(),
                std::fs::read(fixture(&format!("ffv1-gray-8-{index}.packet"))).unwrap()
            );
        }
        let decoded = fvid_media::decode_video(&source).unwrap();
        assert_eq!(
            (
                decoded.video_frames,
                decoded.width,
                decoded.height,
                decoded.decode_errors
            ),
            (4, 4, 3, 0)
        );
        assert_eq!(decoded.backend, "owned Matroska FFV1 decode");
        let interval = fvid_media::decode_video_transformed(
            &source,
            fvid_media::DecodeTransform {
                interval: Some((45000, 125000)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(interval.video_frames, 2);
        let public = fvid_media::probe(&source).unwrap();
        assert_eq!(public.streams[0].start, Some(5000000));
        let mut native =
            fvid::container::webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        native.scan_all().unwrap();
        assert_eq!(native.packets.len(), 4);
        for (owned, front) in reader.packets.iter().zip(&native.packets) {
            assert_eq!(
                (owned.offset, owned.size, owned.pts_ns, owned.duration_ns),
                (front.offset, front.size, front.pts_ns, front.duration_ns)
            );
        }
    }
}
#[test]
fn laced_block_duration_is_split_and_padding_only_applies_to_its_boundary() {
    for padding in [-5000000, 5000000] {
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            std::fs::File::open(fixture(&format!("matroska-lace-group-{padding}.mkv"))).unwrap(),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 4);
        for (i, p) in reader.packets.iter().enumerate() {
            assert_eq!(p.pts_ns, 5000000 + i as i64 * 40250000);
            assert_eq!(p.duration_ns, Some(40250000));
            let boundary = if padding < 0 { 0 } else { 3 };
            assert_eq!(
                p.discard_padding_ns,
                if i == boundary { padding } else { 0 }
            );
        }
    }
}
#[test]
fn malformed_laces_and_index_budget_fail_before_packet_decode() {
    for (name, message) in [
        ("matroska-lace-invalid-fixed.mkv", "unequal frame sizes"),
        ("matroska-lace-invalid-xiph.mkv", "lace sizes exceed block"),
        ("matroska-lace-invalid-ebml.mkv", "lace size difference"),
        (
            "matroska-lace-no-clock.mkv",
            "declared frame or block duration",
        ),
    ] {
        let error = fvid_media::owned_webm::WebmReader::open(
            std::fs::File::open(fixture(name)).unwrap(),
            Default::default(),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains(message), "{name}: {error}");
    }
    let error = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(fixture("matroska-lace-fixed.mkv")).unwrap(),
        fvid_media::owned_webm::Limits {
            packets: 3,
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("packet count exceeds limit"));
}

#[test]
fn matroska_probe_subtracts_codec_delay_without_changing_packet_clock() {
    let source = fixture("matroska-lace-delay.mkv");
    let public = fvid_media::probe(&source).unwrap();
    assert_eq!(public.streams[0].start, Some(-5000000));
    assert_eq!(public.start_us, Some(-5000));
    let native = fvid::native_probe::probe(&source).unwrap();
    assert_eq!(public.streams[0].start, native.streams[0].start);
    let reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&source).unwrap(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(reader.packets[0].pts_ns, 5000000);
}

#[test]
fn opus_laces_without_default_duration_use_each_packet_clock() {
    for name in [
        "opus-lace-xiph.mka",
        "opus-lace-ebml.mka",
        "opus-lace-xiph-group.mka",
        "opus-lace-ebml-group.mka",
    ] {
        let source = fixture(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 4);
        assert_eq!(reader.tracks[0].default_duration_ns, 0);
        let clocks = if name.contains("group") {
            [
                (0, 22500000),
                (22500000, 22500000),
                (45000000, 22500000),
                (67500000, 22500000),
            ]
        } else {
            [
                (0, 20000000),
                (20000000, 40000000),
                (60000000, 10000000),
                (70000000, 20000000),
            ]
        };
        for (i, (pts, duration)) in clocks.into_iter().enumerate() {
            assert_eq!(
                (reader.packets[i].pts_ns, reader.packets[i].duration_ns),
                (pts, Some(duration))
            );
            let coded = fvid_media::owned_opus_packet::duration_ns(&reader.read_packet(i).unwrap())
                .unwrap();
            assert_eq!(coded, [20000000, 40000000, 10000000, 20000000][i]);
        }
        let mut native =
            fvid::container::webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        native.scan_all().unwrap();
        for (owned, front) in reader.packets.iter().zip(&native.packets) {
            assert_eq!(
                (owned.pts_ns, owned.duration_ns),
                (front.pts_ns, front.duration_ns)
            );
        }
        assert_eq!(fvid_media::probe(&source).unwrap().streams[0].codec, "opus");
    }
}

#[test]
fn invalid_opus_lace_framing_is_not_a_codec_timing_fallback() {
    let source = fixture("opus-lace-invalid-packet.mka");
    let error = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&source).unwrap(),
        Default::default(),
    )
    .err()
    .unwrap();
    assert!(!error.is_unsupported());
    assert_eq!(error.to_string(), "invalid Opus packet duration");
    assert_eq!(fvid_media::probe(&source).unwrap_err(), error.to_string());
}
