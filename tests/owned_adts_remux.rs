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
    let packet = [0xff, 0xf1, 0x50, 0x80, 1, 0x3f, 0xfc, 0xe0, 0];
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
    let dry = fvid_control::CopyOptions {
        progress: Some(fvid_control::ProgressHook::new(|_| {
            panic!("dry run emitted execution progress")
        })),
        ..Default::default()
    };
    let plan = fvid_media::plan_remux(&source, &dry).unwrap();
    assert_eq!(plan.streams[0].codec, "aac");
    assert!(plan.steps.iter().any(|step| {
        step.detail
            .contains("2 unchanged AAC packets, 4 payload bytes")
    }));
    assert!(!destination.exists());
    let constrained = fvid_control::CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    assert!(fvid_media::plan_remux(&source, &constrained).is_err());
    assert!(fvid_media::remux(&source, &destination, &constrained).is_err());
    assert!(!destination.exists());
    let stats = fvid_media::remux(&source, &destination, &options).unwrap();
    assert_eq!(stats.backend, "owned Matroska");
    assert_eq!((stats.packets, stats.payload_bytes), (2, 4));
    assert!(events.lock().unwrap().last().unwrap().done);
    let bytes = std::fs::read(&destination).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes.clone()), Default::default())
            .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec, "A_AAC");
    assert_eq!(reader.packets[1].pts_ns, 1024 * 1_000_000_000 / 44100);
    for index in 0..2 {
        assert_eq!(reader.read_packet(index).unwrap(), [0xe0, 0]);
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
    assert!(fvid_media::plan_remux(&source, &Default::default()).is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(destination).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn synthetic_adts_concat_cli_copies_aac_instead_of_decoding_pcm() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let sources = vec![
        fixture.join("adts-concat-a.aac"),
        fixture.join("adts-concat-b.aac"),
    ];
    let dir =
        std::env::temp_dir().join(format!("fvid-adts-concat-dispatch-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let expected = dir.join("expected.mka");
    let actual = dir.join("actual.mkv");
    fvid::native_export::concat_adts_aac(&sources, &expected, None, None).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&actual)
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&actual).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    assert_eq!(
        reader.tracks[0].codec, "A_AAC",
        "ADTS concat must preserve encoded AAC"
    );
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 2);
    assert_eq!(bytes, std::fs::read(&expected).unwrap());
}
