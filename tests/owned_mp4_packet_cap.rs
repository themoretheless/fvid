use std::io::Cursor;

#[test]
fn global_packet_cap_preserves_dts_prefix_payload_and_plan() {
    let bytes = include_bytes!("fixtures/playback-errors/mp4-global-packet-cap.mp4");
    let mut input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(input.tracks().len(), 2);
    let mut expected = Vec::new();
    for (track, data) in input.tracks().iter().enumerate() {
        for index in 0..data.samples.len() {
            let sample = data.samples.get(index).unwrap();
            expected.push((sample.dts, track, index));
        }
    }
    expected.sort_unstable();
    assert!(expected.len() >= 4);
    let folder = std::env::temp_dir().join(format!("fvid-mp4-cap-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(folder.clone());
    let source = folder.join("source.mp4");
    std::fs::write(&source, bytes).unwrap();
    for limit in [0, 1, 3, expected.len() as u64 + 1] {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = events.clone();
        let options = fvid_media::CopyOptions {
            max_packets: Some(limit),
            progress: Some(fvid_media::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event)
            })),
            ..Default::default()
        };
        let destination = folder.join(format!("cap-{limit}.mkv"));
        let plan = fvid_media::plan_remux(&source, &options).unwrap();
        assert!(
            events.lock().unwrap().is_empty(),
            "planning emitted progress"
        );
        let stats = fvid_media::remux(&source, &destination, &options).unwrap();
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .all(|event| event.packets <= limit)
        );
        assert!(events.lock().unwrap().last().unwrap().done);
        let count = (limit as usize).min(expected.len());
        assert_eq!(stats.packets, count as u64);
        assert_eq!(stats.backend, "owned Matroska");
        let mut output = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        output.scan_all().unwrap();
        assert_eq!(output.tracks.len(), 2);
        assert_eq!(output.packets.len(), count);
        let mut payload_bytes = 0;
        for (position, &(_, track, index)) in expected[..count].iter().enumerate() {
            let mut payload = Vec::new();
            input.read_packet(track, index, &mut payload).unwrap();
            let packet = &output.packets[position];
            assert_eq!(packet.track, track as u64 + 1);
            assert_eq!(
                packet.pts_ns,
                (input.tracks()[track].samples.get(index).unwrap().pts as i128 * 1_000_000_000
                    / i128::from(input.tracks()[track].timescale)) as i64
            );
            assert_eq!(output.read_packet(position).unwrap(), payload);
            payload_bytes += payload.len() as u64;
        }
        assert_eq!(stats.payload_bytes, payload_bytes);
        assert!(plan.steps.iter().any(|s| s.detail.contains(&format!(
            "copy {count} compressed packets, {payload_bytes} payload bytes"
        ))));
        let saved = std::fs::read(&destination).unwrap();
        if count == expected.len() {
            let full = folder.join("uncapped.mkv");
            fvid_media::remux(&source, &full, &Default::default()).unwrap();
            assert_eq!(std::fs::read(full).unwrap(), saved);
        }
        assert!(fvid_media::remux(&source, &destination, &options).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), saved);
    }
}

#[test]
fn bounded_prefix_cancellation_does_not_publish() {
    let folder = std::env::temp_dir().join(format!("fvid-mp4-cap-cancel-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let source = folder.join("source.mp4");
    let output = folder.join("cancelled.mkv");
    std::fs::write(
        &source,
        include_bytes!("fixtures/playback-errors/mp4-global-packet-cap.mp4"),
    )
    .unwrap();
    let cancel = fvid_media::CancelFlag::new();
    let flag = cancel.clone();
    let options = fvid_media::CopyOptions {
        max_packets: Some(1),
        cancel: Some(cancel),
        progress: Some(fvid_media::ProgressHook::new(move |event| {
            if event.packets == 1 {
                flag.cancel();
            }
        })),
        ..Default::default()
    };
    let result = fvid_media::remux(&source, &output, &options);
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn intentional_prefix_cannot_finalize_a_poisoned_writer() {
    use fvid_media::owned_matroska::{Encoding, PacketWriter, TrackSpec};
    let input = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/mp4-global-packet-cap.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let track = &input.tracks()[0];
    let spec = TrackSpec {
        encoding: Encoding::Avc {
            configuration: &track.configuration,
            width: track.width.into(),
            height: track.height.into(),
        },
        name: "",
        language: "eng",
    };
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output, &[spec]).unwrap();
    assert!(writer.write_packet(1, 0, 1, true, &[1]).is_err());
    assert!(writer.finish_prefix().is_err());
}
