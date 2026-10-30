use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-implicit-fragment-base")
            .join(n),
    )
    .unwrap()
}
fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn implicit_bases_preserve_both_parser_indices_and_packet_bytes() {
    let packet = bytes("frame.vp9");
    let mut failures = Vec::new();
    for c in cases("valid") {
        let n = c["file"].as_str().unwrap();
        let root = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let owned =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let (mut root, mut owned) = match (root, owned) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                failures.push(format!(
                    "{n}: root={}, owned={}",
                    a.err().map(|e| e.to_string()).unwrap_or_default(),
                    b.err().map(|e| e.to_string()).unwrap_or_default()
                ));
                continue;
            }
        };
        let tracks = c["tracks"].as_array().unwrap();
        assert_eq!(root.tracks().len(), tracks.len());
        assert_eq!(owned.tracks().len(), tracks.len());
        for (track, meta) in tracks.iter().enumerate() {
            assert_eq!(
                root.tracks()[track].id,
                u32::try_from(meta["id"].as_u64().unwrap()).unwrap()
            );
            let offsets = meta["offsets"].as_array().unwrap();
            let count = offsets.len();
            assert_eq!(root.tracks()[track].samples.len(), count);
            assert_eq!(owned.tracks()[track].samples.len(), count);
            assert_eq!(
                (
                    root.tracks()[track].duration,
                    root.tracks()[track].timescale
                ),
                (count as u64, 50)
            );
            for i in (0..count).rev() {
                let a = root.tracks()[track].samples.get(i).unwrap();
                let b = owned.tracks()[track].samples.get(i).unwrap();
                let offset = offsets[i].as_u64().unwrap();
                assert_eq!(
                    (a.offset, a.dts, a.pts, a.duration),
                    (offset, i as u64, i as i64, 1),
                    "{n}"
                );
                assert_eq!(
                    (b.offset, b.dts, b.pts, b.duration),
                    (offset, i as u64, i as i64, 1),
                    "{n}"
                );
                let mut data = Vec::new();
                root.read_packet(track, i, &mut data).unwrap();
                assert_eq!(data, packet);
                owned.read_packet(track, i, &mut data).unwrap();
                assert_eq!(data, packet);
            }
        }
        root.restrict_packet_bytes(1);
        assert!(
            root.read_packet(0, 0, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("packet exceeds budget")
        );
    }
    assert!(
        failures.is_empty(),
        "valid fragment addressing refused: {failures:?}"
    );
}
#[test]
fn unresolved_or_outside_media_bases_are_invalid() {
    for c in cases("invalid") {
        let n = c["file"].as_str().unwrap();
        let expected = c["error"].as_str().unwrap();
        let a = match fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Ok(_) => panic!("{n} accepted"),
            Err(e) => e,
        };
        assert!(matches!(a, fvid::Error::Invalid(_)));
        assert!(a.to_string().contains(expected), "{n}: {a}");
        let b =
            match fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            {
                Ok(_) => panic!("{n} accepted"),
                Err(e) => e,
            };
        assert!(!b.is_unsupported());
        assert!(b.to_string().contains(expected), "{n}: {b}");
    }
}
#[cfg(feature = "player")]
#[test]
fn implicit_fragments_play_seek_rewind_and_reach_eof() {
    use std::time::Duration;
    for c in cases("valid") {
        let n = c["file"].as_str().unwrap();
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let count = c["tracks"].as_array().unwrap().last().unwrap()["offsets"]
            .as_array()
            .unwrap()
            .len() as u128;
        let end = count as u64 * 20;
        assert_eq!(r.duration(), Some(Duration::from_millis(end)), "{n}");
        for i in 0..count {
            assert_eq!(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)), "{n}");
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        for ms in [0, 19, 20, 39, 40, end - 1, end, end + 40] {
            assert_eq!(
                r.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            let i = u128::from(ms.min(end - 1) / 20);
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)), "{n}");
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn invalid_implicit_bases_refuse_before_native_decoding() {
    for c in cases("invalid") {
        let n = c["file"].as_str().unwrap();
        let error =
            match fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20) {
                Ok(_) => panic!("{n} admitted"),
                Err(e) => e,
            };
        assert!(
            error.to_string().contains(c["error"].as_str().unwrap()),
            "{n}: {error}"
        );
    }
}
#[test]
fn implicit_fragment_sample_admission_remains_bounded() {
    for c in cases("valid") {
        let n = c["file"].as_str().unwrap();
        let e = match fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(n)),
            fvid::container::mp4::Limits {
                samples: 2,
                ..Default::default()
            },
        ) {
            Ok(_) => panic!("{n} exceeded sample budget"),
            Err(e) => e,
        };
        assert!(
            e.to_string().contains("sample count exceeds budget"),
            "{n}: {e}"
        );
    }
}
