use std::{io::Cursor, path::PathBuf};
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    for cap in [0, 3] {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = events.clone();
        let options = fvid_media::CopyOptions {
            streams: vec![1],
            max_packets: Some(cap),
            metadata_delete: vec!["TITLE".into(), "artist".into()],
            metadata_set: vec![
                ("title".into(), "Synthetic replacement".into()),
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
            metadata_set: vec![("unsupported-key".into(), "value".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_set: vec![("title".into(), "bad\0value".into())],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_delete: vec!["unsupported-key".into()],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            metadata_set: vec![("title".into(), "value".into()); 65],
            ..Default::default()
        },
        fvid_media::CopyOptions {
            stream_metadata_set: vec![(1, "title".into(), "value".into())],
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
