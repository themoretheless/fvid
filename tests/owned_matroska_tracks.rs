use std::io::Cursor;
#[test]
fn library_multitrack_mux_matches_frontend_bytes() {
    use fvid::container::matroska_write as front;
    use fvid_media::owned_matroska as own;
    let mut expected = Cursor::new(Vec::new());
    let mut a = front::PacketWriter::new(
        &mut expected,
        &[
            front::TrackSpec {
                encoding: front::Encoding::Ffv1V1 {
                    width: 16,
                    height: 8,
                },
                name: "Video",
                language: "und",
            },
            front::TrackSpec {
                encoding: front::Encoding::PcmFloat32 {
                    sample_rate: 48000,
                    channels: 1,
                },
                name: "Audio",
                language: "en",
            },
        ],
    )
    .unwrap();
    a.write_packet(0, 0, 40_000_000, true, &[1, 2, 3]).unwrap();
    a.write_packet(1, 0, 1_000_000, true, &[0; 192]).unwrap();
    a.finish().unwrap();
    let mut actual = Cursor::new(Vec::new());
    let mut b = own::PacketWriter::new(
        &mut actual,
        &[
            own::TrackSpec {
                encoding: own::Encoding::Ffv1V1 {
                    width: 16,
                    height: 8,
                },
                name: "Video",
                language: "und",
            },
            own::TrackSpec {
                encoding: own::Encoding::PcmFloat32 {
                    sample_rate: 48000,
                    channels: 1,
                },
                name: "Audio",
                language: "en",
            },
        ],
    )
    .unwrap();
    b.write_packet(0, 0, 40_000_000, true, &[1, 2, 3]).unwrap();
    b.write_packet(1, 0, 1_000_000, true, &[0; 192]).unwrap();
    b.finish().unwrap();
    assert_eq!(actual.get_ref(), expected.get_ref());
    let dir = std::env::temp_dir().join(format!(
        "fvid-library-matroska-remux-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.mkv");
    let destination = dir.join("copy.mkv");
    std::fs::write(&source, actual.get_ref()).unwrap();
    let options = fvid_control::CopyOptions::default();
    let dry = fvid_control::CopyOptions {
        progress: Some(fvid_control::ProgressHook::new(|_| {
            panic!("plan emitted execution progress")
        })),
        ..Default::default()
    };
    let plan = fvid_media::plan_remux(&source, &dry).unwrap();
    assert_eq!(plan.streams.len(), 2);
    assert_eq!(plan.streams[0].codec, "ffv1");
    assert!(
        plan.steps
            .iter()
            .any(|step| step.detail.contains("2 media packets, 195 payload bytes"))
    );
    assert!(!destination.exists());
    let constrained = fvid_control::CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    assert!(fvid_media::plan_remux(&source, &constrained).is_err());
    assert!(!destination.exists());

    let stats = fvid_media::remux(&source, &destination, &options).unwrap();
    assert_eq!(stats.backend, "owned Matroska");
    assert_eq!(stats.packets, 2);
    assert_eq!(std::fs::read(&destination).unwrap(), *actual.get_ref());
    assert!(fvid_media::remux(&source, &destination, &options).is_err());
    let cancelled = dir.join("cancelled.mkv");
    let cancel = fvid_control::CancelFlag::default();
    cancel.cancel();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    assert!(fvid_media::remux(&source, &cancelled, &options).is_err());
    assert!(!cancelled.exists());
    let during = dir.join("during.mkv");
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
    assert!(fvid_media::remux(&source, &during, &options).is_err());
    assert!(!during.exists());

    let audio = dir.join("audio.mka");
    assert!(fvid_media::remux(&source, &audio, &Default::default()).is_err());
    assert!(!audio.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(destination).unwrap();
    std::fs::remove_dir(dir).unwrap();

    let mut copied = Vec::new();
    let copied_stats = fvid_media::owned_matroska_copy::copy(
        &mut Cursor::new(actual.get_ref()),
        &mut copied,
        false,
        None,
        None,
    )
    .unwrap();
    assert_eq!(copied, *actual.get_ref());
    assert_eq!(copied_stats.packets, 2);
    assert_eq!(copied_stats.payload_bytes, 195);
    let mut refused_copy = Vec::new();
    assert!(
        fvid_media::owned_matroska_copy::copy(
            &mut Cursor::new(actual.get_ref()),
            &mut refused_copy,
            true,
            None,
            None,
        )
        .is_err()
    );
    assert!(refused_copy.is_empty());

    let mut parsed = fvid_media::owned_webm::WebmReader::open(actual, Default::default()).unwrap();
    parsed.scan_all().unwrap();
    assert_eq!(parsed.tracks.len(), 2);
    assert_eq!(parsed.tracks[1].name, "Audio");
    assert_eq!(parsed.read_packet(1).unwrap(), vec![0; 192]);
    let mut refused = Cursor::new(Vec::new());
    assert!(
        own::PacketWriter::new(
            &mut refused,
            &[own::TrackSpec {
                encoding: own::Encoding::Avc {
                    configuration: &[],
                    width: 16,
                    height: 8
                },
                name: "",
                language: ""
            },]
        )
        .is_err()
    );
    assert!(refused.get_ref().is_empty());
}
