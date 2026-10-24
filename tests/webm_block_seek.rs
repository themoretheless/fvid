use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/webm-block-seek")
}

#[test]
fn malformed_blockgroups_and_reserved_block_flags_are_rejected() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let expected_err = case["expected_error"].as_str().unwrap();
        let path = root().join(file);
        let data = fs::read(&path).unwrap();

        let err = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(&data),
            Default::default(),
        )
        .and_then(|mut r| {
            r.scan_all()?;
            Ok(r)
        })
        .err()
        .unwrap_or_else(|| panic!("WebmReader accepted {file}"));
        assert!(
            !err.is_unsupported() && err.to_string().contains(expected_err),
            "{file}: {err}"
        );

        let player_err = fvid::playback_native::NativeReader::software(
            Cursor::new(&data),
            32 << 20,
        )
        .err()
        .unwrap_or_else(|| panic!("NativeReader accepted {file}"));
        assert!(
            matches!(player_err, fvid::Error::Invalid(_))
                && player_err.to_string().contains(expected_err),
            "{file}: {player_err:?}"
        );

        assert!(fvid_media::decode_video(&path).is_err(), "{file}");
    }
}

#[test]
fn equal_pts_keyframes_and_delayed_audio_tracks_seek_deterministically() {
    // 1. Valid BlockGroup control
    let bg_data = fs::read(root().join("valid-blockgroup.mkv")).unwrap();
    let mut bg_reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bg_data), Default::default())
            .unwrap();
    bg_reader.scan_all().unwrap();
    assert_eq!(bg_reader.packets.len(), 1);
    assert!(bg_reader.packets[0].keyframe);
    assert_eq!(bg_reader.packets[0].duration_ns, Some(40_000_000));

    // 2. Equal-PTS VP9 invisible setup keyframe + show-existing keyframe seek
    let vp9_data = fs::read(root().join("valid-equal-pts-vp9-seek.webm")).unwrap();
    let expected_planes = fs::read(root().join("expected-16x16.yuv420p16le")).unwrap();

    let mut player =
        fvid::playback_native::NativeReader::software(Cursor::new(&vp9_data), 32 << 20).unwrap();
    player.seek(Duration::from_millis(40)).unwrap();
    assert_eq!(
        player.frame_interval(),
        Some((40_000_000, 80_000_000, 1_000_000_000))
    );

    // Also verify raw decoded planes after seeking to 40 ms via WebmVideoReader
    let mut seq =
        fvid::playback_native::NativeReader::software(Cursor::new(&vp9_data), 32 << 20).unwrap();
    let _first = seq.read_frame_raw().unwrap().unwrap();
    let second = seq.read_frame_raw().unwrap().unwrap();
    let second_bytes: Vec<u8> = match second {
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        fvid::playback_native::RawFrame::Planar8(p) => [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()]
            .concat()
            .into_iter()
            .flat_map(|v| u16::from(v).to_le_bytes())
            .collect(),
        _ => panic!("expected planar VP9 frame"),
    };
    assert_eq!(second_bytes, expected_planes);

    // 3. Multi-track delayed audio origin + equal-PTS packet seek in WebmAudioReader
    #[cfg(feature = "player")]
    {
        let mt_data = fs::read(root().join("valid-multitrack-audio-seek.mkv")).unwrap();
        let mut audio = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(&mt_data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(audio.track().number, 2);

        // Seeking before the first packet of track 2 (which starts at 50_000_000 ns)
        // must land on track 2's first packet and return its PTS (50_000_000), not (0, 0).
        let landed_before = audio.seek(0);
        assert_eq!(landed_before, 50_000_000);
        let pkt0 = audio.read_packet().unwrap().unwrap();
        assert_eq!(pkt0.pts_ns, 50_000_000);
        assert_eq!(
            pkt0.data,
            [1000i16, 2000, 3000, 4000]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<u8>>()
        );

        // Seeking to 50_000_000 ns where two packets share the same PTS must select the FIRST
        // packet at 50_000_000 ns so both packets are read in sequence.
        let landed_exact = audio.seek(50_000_000);
        assert_eq!(landed_exact, 50_000_000);
        let first_at_50 = audio.read_packet().unwrap().unwrap();
        let second_at_50 = audio.read_packet().unwrap().unwrap();
        assert_eq!(first_at_50.pts_ns, 50_000_000);
        assert_eq!(second_at_50.pts_ns, 50_000_000);
        assert_eq!(
            first_at_50.data,
            [1000i16, 2000, 3000, 4000]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<u8>>()
        );
        assert_eq!(
            second_at_50.data,
            [5000i16, 6000, 7000, 8000]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<u8>>()
        );
        assert!(audio.read_packet().unwrap().is_none());
    }
}
