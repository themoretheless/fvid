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
