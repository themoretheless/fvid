use std::{io::Cursor, path::PathBuf};
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn invalid_low_level_track_overrides_emit_no_header() {
    use fvid_media::owned_mp4_matroska::{RemuxMetadata, TrackMetadataOverride};
    let bytes = include_bytes!("fixtures/playback-errors/mp4-track-tag-edits.mp4");
    for edits in [
        vec![TrackMetadataOverride {
            index: 1,
            name: Some("unselected"),
            language: None,
        }],
        vec![TrackMetadataOverride {
            index: 99,
            name: None,
            language: Some("eng"),
        }],
        vec![
            TrackMetadataOverride {
                index: 0,
                name: Some("first"),
                language: None,
            },
            TrackMetadataOverride {
                index: 0,
                name: Some("duplicate"),
                language: None,
            },
        ],
        vec![TrackMetadataOverride {
            index: 0,
            name: Some("bad\0title"),
            language: None,
        }],
    ] {
        let mut input =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let metadata = fvid_media::owned_matroska::FileMetadata::from_mp4(&input);
        let mut output = Cursor::new(Vec::new());
        assert!(
            fvid_media::owned_mp4_matroska::write_selected_with_metadata_overrides(
                &mut input,
                &mut output,
                None,
                None,
                Some(0),
                &[0],
                RemuxMetadata {
                    file: &metadata,
                    tracks: &edits
                }
            )
            .unwrap_err()
            .to_string()
            .contains("track metadata override")
        );
        assert!(output.into_inner().is_empty());
    }
}
#[test]
fn track_title_and_language_edits_use_original_indexes_after_reordering() {
    let folder = Directory(
        std::env::temp_dir().join(format!("fvid-mp4-track-edits-{}", std::process::id())),
    );
    std::fs::create_dir(&folder.0).unwrap();
    let bytes = include_bytes!("fixtures/playback-errors/mp4-track-tag-edits.mp4");
    let source = folder.0.join("source.mp4");
    std::fs::write(&source, bytes).unwrap();
    let mut input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(
        (
            input.tracks()[0].name.as_str(),
            input.tracks()[0].language.as_str()
        ),
        ("Synthetic track 0", "eng")
    );
    assert_eq!(
        (
            input.tracks()[1].name.as_str(),
            input.tracks()[1].language.as_str()
        ),
        ("Synthetic track 1", "fra")
    );
    let options = fvid_media::CopyOptions {
        streams: vec![1, 0],
        max_packets: Some(3),
        stream_metadata_delete: vec![(0, "TITLE".into()), (0, "language".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "Transient track 0 title".into()),
            (1, "title".into(), "Discarded title".into()),
            (1, "TITLE".into(), "Дорожка один".into()),
        ],
        ..Default::default()
    };
    let plan = fvid_media::owned_mp4_remux::plan_remux(&source, &options).unwrap();
    assert_eq!(
        plan.streams
            .iter()
            .map(|stream| stream.index)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    let destination = folder.0.join("edited.mkv");
    let stats = fvid_media::remux(&source, &destination, &options).unwrap();
    assert_eq!(stats.backend, "owned Matroska");
    let mut output = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&destination).unwrap()),
        Default::default(),
    )
    .unwrap();
    output.scan_all().unwrap();
    assert_eq!(
        (
            output.tracks[0].name.as_str(),
            output.tracks[0].language.as_str()
        ),
        ("Дорожка один", "fra")
    );
    assert_eq!(
        (
            output.tracks[1].name.as_str(),
            output.tracks[1].language.as_str()
        ),
        ("", "")
    );
    assert_eq!(output.tags, *input.tags());
    let mut expected = Vec::new();
    for (track, data) in input.tracks().iter().enumerate() {
        for index in 0..data.samples.len() {
            expected.push((data.samples.get(index).unwrap().dts, track, index));
        }
    }
    expected.sort_unstable();
    assert_eq!(output.packets.len(), 3);
    for (position, &(_, source_track, index)) in expected[..3].iter().enumerate() {
        assert_eq!(
            output.packets[position].track,
            if source_track == 1 { 1 } else { 2 }
        );
        let mut packet = Vec::new();
        input.read_packet(source_track, index, &mut packet).unwrap();
        assert_eq!(output.read_packet(position).unwrap(), packet);
    }
    let language_options = fvid_media::CopyOptions {
        streams: vec![1],
        max_packets: Some(0),
        stream_metadata_set: vec![(1, "LANGUAGE".into(), "rus".into())],
        progress: Some(fvid_media::ProgressHook::new(|_| {
            panic!("a refused public zero-limit request must emit no progress")
        })),
        ..Default::default()
    };
    let language_output = folder.0.join("language.mkv");
    for error in [
        fvid_media::owned_mp4_remux::plan_remux(&source, &language_options).unwrap_err(),
        fvid_media::remux(&source, &language_output, &language_options).unwrap_err(),
    ] {
        assert!(error.contains("packet work limit requires at least one packet"));
    }
    assert!(!language_output.exists());
    // Header-only output belongs to the explicit low-level writer, rather
    // than a public CopyOptions request with an invalid zero work limit.
    let mut header_input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let header_metadata = fvid_media::owned_matroska::FileMetadata::from_mp4(&header_input);
    let language_edit = fvid_media::owned_mp4_matroska::TrackMetadataOverride {
        index: 1,
        name: None,
        language: Some("rus"),
    };
    let mut header = Cursor::new(Vec::new());
    let event = fvid_media::owned_mp4_matroska::write_selected_with_metadata_overrides(
        &mut header_input,
        &mut header,
        None,
        None,
        Some(0),
        &[1],
        fvid_media::owned_mp4_matroska::RemuxMetadata {
            file: &header_metadata,
            tracks: &[language_edit],
        },
    )
    .unwrap();
    assert_eq!(event.packets, 0);
    let mut language = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(header.into_inner()),
        Default::default(),
    )
    .unwrap();
    language.scan_all().unwrap();
    assert_eq!(
        (
            language.tracks[0].name.as_str(),
            language.tracks[0].language.as_str()
        ),
        ("Synthetic track 1", "rus")
    );
    assert!(language.packets.is_empty());
    for (case, options) in [
        fvid_media::CopyOptions {
            streams: vec![1],
            stream_metadata_set: vec![(0, "title".into(), "unselected".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            stream_metadata_delete: vec![(2, "title".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            stream_metadata_set: vec![(0, "language".into(), "bad\0language".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            stream_metadata_set: vec![(1, "title".into(), "excess".into()); 65],
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let destination = folder.0.join(format!("invalid-track-{case}.mkv"));
        assert!(fvid_media::owned_mp4_remux::plan_remux(&source, &options).is_err());
        assert!(fvid_media::owned_mp4_remux::remux(&source, &destination, &options).is_err());
        assert!(!destination.exists());
    }
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}
#[test]
fn container_tag_edits_keep_selected_packet_prefix_and_source_metadata() {
    let folder =
        Directory(std::env::temp_dir().join(format!("fvid-mp4-tag-edits-{}", std::process::id())));
    std::fs::create_dir(&folder.0).unwrap();
    let source = folder.0.join("source.mp4");
    let bytes = include_bytes!("fixtures/playback-errors/mp4-container-tag-edits.mp4");
    std::fs::write(&source, bytes).unwrap();
    let mut input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(input.tags().title, "Synthetic original title");
    assert_eq!(input.tags().artist, "Synthetic original artist");
    let public_zero = fvid_media::CopyOptions {
        streams: vec![1],
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_destination = folder.0.join("public-zero.mkv");
    for error in [
        fvid_media::owned_mp4_remux::plan_remux(&source, &public_zero).unwrap_err(),
        fvid_media::remux(&source, &zero_destination, &public_zero).unwrap_err(),
    ] {
        assert!(error.contains("packet work limit requires at least one packet"));
    }
    assert!(!zero_destination.exists());
    let mut header_input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut header_metadata = fvid_media::owned_matroska::FileMetadata::from_mp4(&header_input);
    header_metadata.tags.title = "Synthetic replacement".into();
    header_metadata.tags.artist.clear();
    header_metadata.tags.comment = "Тест".into();
    let mut header = Cursor::new(Vec::new());
    let event = fvid_media::owned_mp4_matroska::write_selected_with_metadata_overrides(
        &mut header_input,
        &mut header,
        None,
        None,
        Some(0),
        &[1],
        fvid_media::owned_mp4_matroska::RemuxMetadata {
            file: &header_metadata,
            tracks: &[],
        },
    )
    .unwrap();
    assert_eq!(event.packets, 0);
    let mut header = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(header.into_inner()),
        Default::default(),
    )
    .unwrap();
    header.scan_all().unwrap();
    assert_eq!(header.tags.title, "Synthetic replacement");
    assert!(header.tags.artist.is_empty());
    assert_eq!(header.tags.album, "Synthetic retained album");
    assert_eq!(header.tags.comment, "Тест");
    assert_eq!(header.tracks.len(), 1);
    assert!(header.packets.is_empty());
    for cap in [1, 3] {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = events.clone();
        let options = fvid_media::CopyOptions {
            streams: vec![1],
            max_packets: Some(cap),
            metadata_delete: vec!["artist".into()],
            metadata_set: vec![
                ("title".into(), "Synthetic replacement".into()),
                ("artist".into(), "Transient artist".into()),
                ("comment".into(), "Тест".into()),
            ],
            progress: Some(fvid_media::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event)
            })),
            ..Default::default()
        };
        let plan = fvid_media::owned_mp4_remux::plan_remux(&source, &options).unwrap();
        assert_eq!(plan.streams[0].index, 1);
        assert!(events.lock().unwrap().is_empty());
        let destination = folder.0.join(format!("cap-{cap}.mkv"));
        let result = fvid_media::remux(&source, &destination, &options).unwrap();
        assert_eq!(result.backend, "owned Matroska");
        assert_eq!(result.packets, cap);
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.done)
                .count(),
            1
        );
        assert!(events.lock().unwrap().last().unwrap().done);
        let mut output = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        output.scan_all().unwrap();
        assert_eq!(output.packets.len(), cap as usize);
        assert_eq!(output.tags.title, "Synthetic replacement");
        assert!(output.tags.artist.is_empty());
        assert_eq!(output.tags.album, "Synthetic retained album");
        assert_eq!(output.tags.comment, "Тест");
        assert_eq!(output.tracks.len(), 1);
        for index in 0..output.packets.len() {
            let actual = output.read_packet(index).unwrap();
            let mut expected = Vec::new();
            input.read_packet(1, index, &mut expected).unwrap();
            assert_eq!(actual, expected);
        }
        assert!(fvid_media::remux(&source, &destination, &options).is_err());
    }
    assert_eq!(input.tags().title, "Synthetic original title");
    let cancel = fvid_media::CancelFlag::new();
    let trigger = cancel.clone();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = events.clone();
    let options = fvid_media::CopyOptions {
        streams: vec![1],
        max_packets: Some(3),
        cancel: Some(cancel),
        metadata_set: vec![("title".into(), "Cancelled replacement".into())],
        progress: Some(fvid_media::ProgressHook::new(move |event| {
            seen.lock().unwrap().push(event);
            if event.packets == 1 {
                trigger.cancel();
            }
        })),
        ..Default::default()
    };
    let destination = folder.0.join("cancelled.mkv");
    assert!(
        fvid_media::owned_mp4_remux::remux(&source, &destination, &options)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(!destination.exists());
    assert!(events.lock().unwrap().iter().all(|event| !event.done));
    assert!(std::fs::read_dir(&folder.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
    for (case, options) in [
        fvid_media::CopyOptions {
            metadata_set: vec![("".into(), "value".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_set: vec![("title".into(), "bad\0value".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_delete: vec!["bad\0key".into()],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_set: vec![("title".into(), "value".into()); 65],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            stream_metadata_set: vec![(1, "rotate".into(), "value".into())],
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let destination = folder.0.join(format!("invalid-{case}.mkv"));
        assert!(fvid_media::owned_mp4_remux::plan_remux(&source, &options).is_err());
        assert!(fvid_media::owned_mp4_remux::remux(&source, &destination, &options).is_err());
        assert!(!destination.exists());
    }
}
