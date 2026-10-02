use std::io::Cursor;
#[test]
fn library_mp4_demux_matches_frontend_indexes_metadata_and_packets() {
    for name in [
        "video.mp4",
        "audio.mp4",
        "fragmented/video.mp4",
        "fragmented/audio.mp4",
        "hevc/hdr10.mp4",
        "hevc/mdcv-only.mp4",
        "hevc/main10-ipb.mp4",
        "short/avc-baseline.mp4",
        "short/avc-bframes.mp4",
        "tracks/named.mp4",
        "tags/tags.mp4",
        "subtitles/mov-text-tracks.mp4",
        "alac/stereo-24.m4a",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&path).unwrap();
        let mut front =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut own =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!(
            format!("{:?}", own.tracks()),
            format!("{:?}", front.tracks()),
            "{name}: track/index metadata"
        );
        assert_eq!(own.tags(), front.tags(), "{name}: file tags");
        assert_eq!(
            format!("{:?}", own.chapters()),
            format!("{:?}", front.chapters()),
            "{name}: chapters"
        );
        assert_eq!(
            format!("{:?}", own.refused()),
            format!("{:?}", front.refused()),
            "{name}: skipped tracks"
        );
        assert_eq!(own.movie_timescale(), front.movie_timescale());
        for index in 0..own.tracks().len() {
            let count = own.tracks()[index].samples.len();
            if count == 0 {
                continue;
            }
            // Seek forwards, then backwards and reread, including a reset cursor.
            for sample in [0, count - 1, count / 2, 0] {
                let mut expected = Vec::new();
                let mut actual = Vec::new();
                front.read_packet(index, sample, &mut expected).unwrap();
                own.read_packet(index, sample, &mut actual).unwrap();
                assert_eq!(actual, expected, "{name}: stream {index}, sample {sample}");
                own.invalidate_position();
                front.invalidate_position();
            }
        }
    }
}
#[test]
fn library_mp4_preserves_recognition_and_specific_extent_refusal() {
    for kind in [
        b"ftyp", b"moov", b"mdat", b"wide", b"free", b"skip", b"uuid",
    ] {
        let bytes = [8u32.to_be_bytes().as_slice(), kind.as_slice()].concat();
        assert!(fvid_media::owned_mp4::recognizes_prefix(&bytes));
    }
    assert!(!fvid_media::owned_mp4::recognizes_prefix(
        b"\xff\xd8\xff\xe0\0\x10JF"
    ));
    let bytes = [100u32.to_be_bytes().as_slice(), b"ftyp", b"isom"].concat();
    let error = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default())
        .err()
        .unwrap()
        .to_string();
    let front = fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default())
        .err()
        .unwrap()
        .to_string();
    assert_eq!(error, front);
    assert!(error.contains("Invalid MP4 box"));
    assert!(error.contains("declared size 100"));
    assert!(error.contains("remaining file bytes 12"));
}

#[test]
fn library_mp4_enforces_metadata_and_packet_limits_before_allocating_payload() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let bytes = std::fs::read(source).unwrap();
    let own = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(&bytes),
        fvid_media::owned_mp4::Limits {
            metadata_bytes: 1,
            ..Default::default()
        },
    )
    .err()
    .unwrap()
    .to_string();
    let front = fvid::container::mp4::Mp4Reader::open(
        Cursor::new(&bytes),
        fvid::container::mp4::Limits {
            metadata_bytes: 1,
            ..Default::default()
        },
    )
    .err()
    .unwrap()
    .to_string();
    assert_eq!(own, front);
    assert!(own.contains("metadata exceeds budget"));
    let mut own = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(&bytes),
        fvid_media::owned_mp4::Limits {
            packet_bytes: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut front = fvid::container::mp4::Mp4Reader::open(
        Cursor::new(&bytes),
        fvid::container::mp4::Limits {
            packet_bytes: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut actual = vec![17, 23];
    let mut expected = actual.clone();
    let error = own.read_packet(0, 0, &mut actual).unwrap_err().to_string();
    assert_eq!(
        error,
        front
            .read_packet(0, 0, &mut expected)
            .unwrap_err()
            .to_string()
    );
    assert!(error.contains("packet exceeds budget"));
    assert_eq!(actual, [17, 23]);
    assert_eq!(expected, actual);
    assert!(own
        .read_packet(usize::MAX, 0, &mut actual)
        .unwrap_err()
        .to_string()
        .contains("index out of range"));
    assert_eq!(actual, [17, 23]);
}

#[test]
fn owned_mp4_matroska_mux_and_publication_match_frontend_bytes() {
    let dir = std::env::temp_dir().join(format!("fvid-library-mp4-remux-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    for name in [
        "video.mp4",
        "audio.mp4",
        "hevc/main-ipb.mp4",
        "hevc/hdr10.mp4",
        "short/avc-bframes.mp4",
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut front =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut expected = Cursor::new(Vec::new());
        let front_event =
            fvid::container::mp4_matroska::write(&mut front, &mut expected, None, None).unwrap();
        let mut own =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let mut actual = Cursor::new(Vec::new());
        let own_event =
            fvid_media::owned_mp4_matroska::write(&mut own, &mut actual, None, None).unwrap();
        assert_eq!(own_event, front_event);
        assert_eq!(
            actual.get_ref(),
            expected.get_ref(),
            "{name}: packet mux bytes"
        );
        let output = dir.join(format!(
            "{}.{}",
            name.replace('/', "-"),
            if name == "audio.mp4" { "mka" } else { "mkv" }
        ));
        let options = fvid_control::CopyOptions {
            progress: Some(fvid_control::ProgressHook::new(|_| {
                panic!("read-only plan emitted progress")
            })),
            ..Default::default()
        };
        let plan = fvid_media::plan_remux(&source, &options).unwrap();
        assert!(plan
            .notes
            .iter()
            .any(|note| note.contains("owned MP4/Matroska")));
        assert!(plan.steps.iter().any(|step| step
            .detail
            .contains(&format!("{} compressed packets", own_event.packets))));
        assert!(!output.exists());
        let stats = fvid_media::remux(&source, &output, &Default::default()).unwrap();
        assert_eq!(stats.backend, "owned Matroska");
        assert_eq!(
            (stats.packets, stats.payload_bytes),
            (own_event.packets, own_event.payload_bytes)
        );
        assert_eq!(std::fs::read(&output).unwrap(), expected.into_inner());
        assert!(
            fvid_media::owned_mp4_remux::remux(&source, &output, &Default::default())
                .unwrap_err()
                .contains("already exists")
        );
    }
}

#[test]
fn library_mp4_remux_cancellation_and_packet_cap_leave_no_partial_file() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let dir = std::env::temp_dir().join(format!("fvid-mp4-remux-controls-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let output = dir.join("cancelled.mkv");
    let cancel = fvid_control::CancelFlag::default();
    let captured = cancel.clone();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets > 0 {
                captured.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_mp4_remux::remux(&source, &output, &options)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let capped = dir.join("capped.mkv");
    assert!(fvid_media::owned_mp4_remux::remux(
        &source,
        &capped,
        &fvid_control::CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("packet exceeds budget"));
    assert!(!capped.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
