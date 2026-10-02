use std::{
    io::Cursor,
    sync::{Arc, Mutex},
};
#[test]
fn synthetic_adts_file_remux_preserves_packets_and_publishes_atomically() {
    let dir = std::env::temp_dir().join(format!("fvid-library-adts-remux-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.aac");
    let destination = dir.join("copy.mka");
    let packet = [0xff, 0xf1, 0x50, 0x80, 1, 0x1f, 0xfc, 0xe0];
    std::fs::write(&source, [packet, packet].concat()).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let published = destination.clone();
    let options = fvid_control::CopyOptions {
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
            }
            captured.lock().unwrap().push(event);
        })),
        ..Default::default()
    };
    let stats = fvid_media::remux(&source, &destination, &options).unwrap();
    assert_eq!(stats.backend, "owned Matroska");
    assert_eq!((stats.packets, stats.payload_bytes), (2, 2));
    assert!(events.lock().unwrap().last().unwrap().done);
    let bytes = std::fs::read(&destination).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes.clone()), Default::default())
            .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec, "A_AAC");
    assert_eq!(reader.packets[1].pts_ns, 1024 * 1_000_000_000 / 44100);
    for index in 0..2 {
        assert_eq!(reader.read_packet(index).unwrap(), [0xe0]);
    }
    assert!(fvid_media::remux(&source, &destination, &options).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    let failed = dir.join("failed.mka");
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
    assert!(fvid_media::remux(&source, &failed, &options).is_err());
    assert!(!failed.exists());
    std::fs::write(&source, [&packet[..], &packet[..7]].concat()).unwrap();
    assert!(fvid_media::remux(&source, &failed, &Default::default()).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(destination).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
