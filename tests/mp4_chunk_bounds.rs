use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-chunk-bounds")
}

#[test]
fn overlapping_or_non_increasing_chunks_zero_size_samples_and_empty_tracks_are_rejected() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let expected_err = case["expected_error"].as_str().unwrap();
        let path = root().join(file);
        let data = fs::read(&path).unwrap();

        let root_err =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default())
                .err()
                .unwrap_or_else(|| panic!("RootMp4Reader accepted {file}"));
        assert!(
            matches!(root_err, fvid::Error::Invalid(_)),
            "{file}: {root_err:?}"
        );
        assert!(
            root_err.to_string().contains(expected_err),
            "{file}: {root_err}"
        );

        let owned_err =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default())
                .err()
                .unwrap_or_else(|| panic!("OwnedMp4Reader accepted {file}"));
        assert!(
            !owned_err.is_unsupported() && owned_err.to_string().contains(expected_err),
            "{file}: {owned_err}"
        );

        assert!(
            fvid_media::owned_mp4_probe::probe_mp4(&path).is_err(),
            "{file}"
        );
        assert!(fvid_media::decode_video(&path).is_err(), "{file}");
    }
}

#[test]
fn chunk_ordering_non_empty_tracks_and_positive_sample_sizes_are_enforced() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let expected_packet = fs::read(root().join("frame.vp9")).unwrap();

    for control in manifest["controls"].as_array().unwrap() {
        let file = control["file"].as_str().unwrap();
        let path = root().join(file);
        let data = fs::read(&path).unwrap();

        let mut root_reader =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let mut owned_reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        assert_eq!(root_reader.tracks().len(), 1);
        assert_eq!(owned_reader.tracks().len(), 1);
        assert_eq!(root_reader.tracks()[0].samples.len(), 3);
        assert_eq!(owned_reader.tracks()[0].samples.len(), 3);

        for sample_idx in 0..3 {
            let mut a = Vec::new();
            let mut b = Vec::new();
            root_reader.read_packet(0, sample_idx, &mut a).unwrap();
            owned_reader.read_packet(0, sample_idx, &mut b).unwrap();
            assert_eq!(a, expected_packet, "{file} sample {sample_idx}");
            assert_eq!(b, expected_packet, "{file} sample {sample_idx}");
        }

        let stats = fvid_media::decode_video(&path).unwrap();
        assert_eq!((stats.width, stats.height, stats.video_frames), (16, 16, 3));

        let mut player =
            fvid::playback_native::NativeReader::software(Cursor::new(&data), 32 << 20).unwrap();
        for index in 0..3 {
            let rgb = player
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap();
            assert_eq!(rgb, vec![130; 16 * 16 * 3], "{file}");
            assert_eq!(player.frame_interval().unwrap(), (index, index + 1, 50));
        }
        assert!(player.read_frame_raw().unwrap().is_none());
        player.rewind().unwrap();
        assert!(player.seek_raw(Duration::from_millis(20)).unwrap().is_some());
        assert_eq!(player.frame_interval().unwrap(), (1, 2, 50));
    }

    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let expected_err = case["expected_error"].as_str().unwrap();
        let path = root().join(file);
        let data = fs::read(&path).unwrap();

        let root_err =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default())
                .err()
                .unwrap_or_else(|| panic!("RootMp4Reader accepted {file}"));
        assert!(
            matches!(root_err, fvid::Error::Invalid(_)),
            "{file}: {root_err:?}"
        );
        assert!(
            root_err.to_string().contains(expected_err),
            "{file}: {root_err}"
        );

        let owned_err =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default())
                .err()
                .unwrap_or_else(|| panic!("OwnedMp4Reader accepted {file}"));
        assert!(
            !owned_err.is_unsupported() && owned_err.to_string().contains(expected_err),
            "{file}: {owned_err}"
        );

        assert!(
            fvid_media::owned_mp4_probe::probe_mp4(&path).is_err(),
            "{file}"
        );
        assert!(fvid_media::decode_video(&path).is_err(), "{file}");
    }
}
