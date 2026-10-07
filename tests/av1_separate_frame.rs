use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn check_scaled_webm(r: &serde_json::Value) {
    let name = r["webm"].as_str().unwrap();
    let data = std::fs::read(root().join(name)).unwrap();
    let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
    let mut reader =
        fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 16 << 20).unwrap();
    let frame_bytes = expected.len() / 2;
    for _ in 0..2 {
        for i in 0..2 {
            let f = reader
                .read_frame_planes()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(
                f.y.iter()
                    .chain(&f.cb)
                    .chain(&f.cr)
                    .copied()
                    .collect::<Vec<_>>(),
                expected[i * frame_bytes..(i + 1) * frame_bytes],
                "{name} {i}"
            );
            assert_eq!(
                reader.dimensions(),
                [
                    r["current_size"][0].as_u64().unwrap() as usize,
                    r["current_size"][1].as_u64().unwrap() as usize
                ]
            );
            assert_eq!(
                reader.frame_interval(),
                Some((
                    i as u128 * 20_000_000,
                    (i as u128 + 1) * 20_000_000,
                    1_000_000_000
                ))
            );
        }
        assert!(reader.read_frame_planes().unwrap().is_none());
        reader.rewind();
    }
    assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
    let first = reader.read_frame_planes().unwrap().unwrap();
    assert_eq!(
        first
            .y
            .iter()
            .chain(&first.cb)
            .chain(&first.cr)
            .copied()
            .collect::<Vec<_>>(),
        expected[..frame_bytes],
        "{name}: seek pixels"
    );
}

#[test]
fn separate_frame_pixels_maps_reset_webm_and_seek_match_oracle() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-separate-frame-generated.json")).unwrap(),
    )
    .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 96);
    for case in 0..12 {
        assert_eq!(records.iter().filter(|r| r["case"] == case).count(), 8);
    }
    for r in records {
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 3, "{name}");
            assert!(!frames[0].show && frames[1].show && frames[2].show);
            for (index, frame) in frames.iter().enumerate() {
                assert_eq!(frame.picture.depth, 8);
                let size = if index == 0 {
                    &r["reference_size"]
                } else {
                    &r["current_size"]
                };
                assert_eq!(
                    frame.picture.size,
                    [
                        size[0].as_u64().unwrap() as u32,
                        size[1].as_u64().unwrap() as u32
                    ],
                    "{name}: dimensions"
                );
                let map: Vec<u8> = r["maps"][index]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(frame.picture.segment_ids, map, "{name} frame {index}");
            }
            let actual: Vec<u8> = frames
                .iter()
                .filter(|f| f.show)
                .flat_map(|f| {
                    f.picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().map(|v| *v as u8))
                })
                .collect();
            assert_eq!(actual, expected, "{name}");
            decoder.reset();
        }
        let mut decoder = Decoder::new(16 << 20);
        let mut incremental = Vec::new();
        for obu in fvid::codec::av1::Obus::new(&bytes) {
            let obu = obu.unwrap();
            let packet = packet(obu.kind, obu.temporal_id, obu.spatial_id, obu.payload);
            let frames = decoder.decode_packet(&packet).unwrap();
            if obu.kind != 4 {
                assert!(frames.is_empty(), "{name}: premature publication");
            }
            incremental.extend(frames);
        }
        assert_eq!(incremental.len(), 3);
        let pixels: Vec<u8> = incremental
            .iter()
            .filter(|f| f.show)
            .flat_map(|f| {
                f.picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().map(|v| *v as u8))
            })
            .collect();
        assert_eq!(pixels, expected, "{name}: packet boundaries");
        check_scaled_webm(r);
    }
}

fn packet(kind: u8, temporal: u8, spatial: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![(kind << 3) | 6, (temporal << 5) | (spatial << 3)];
    let mut size = payload.len();
    loop {
        let byte = (size & 127) as u8;
        size >>= 7;
        bytes.push(byte | if size > 0 { 128 } else { 0 });
        if size == 0 {
            break;
        }
    }
    bytes.extend_from_slice(payload);
    bytes
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("av1-separate-frame-generated.json")).unwrap(),
    )
    .unwrap()
}
#[test]
fn malformed_separate_frames_have_specific_errors() {
    let m = manifest();
    assert_eq!(m["refusals"].as_array().unwrap().len(), 10);
    for r in m["refusals"].as_array().unwrap() {
        let mut d = Decoder::new(16 << 20);
        let error = d
            .decode_packet(&std::fs::read(root().join(r["file"].as_str().unwrap())).unwrap())
            .err()
            .expect("malformed stream accepted")
            .to_string();
        assert!(
            error.contains(r["error"].as_str().unwrap()),
            "{}: {error}",
            r["file"]
        );
        assert!(d
            .decode_packet(&[])
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset"));
    }
}
#[test]
fn multiple_groups_publish_only_a_complete_picture_and_match_pixels() {
    let m = manifest();
    assert_eq!(m["multitile"].as_array().unwrap().len(), 4);
    for r in m["multitile"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(r["file"].as_str().unwrap())).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut d = Decoder::new(16 << 20);
        for _ in 0..2 {
            let mut count = 0;
            for o in fvid::codec::av1::Obus::new(&bytes) {
                let o = o.unwrap();
                if o.kind == 4 {
                    count += 1;
                }
                let frames = d
                    .decode_packet(&packet(o.kind, o.temporal_id, o.spatial_id, o.payload))
                    .unwrap();
                if o.kind != 4 || count < r["groups"].as_u64().unwrap() {
                    assert!(frames.is_empty(), "partial picture was published");
                } else {
                    assert_eq!(frames.len(), 1);
                    assert!(frames[0].show);
                    assert_eq!(
                        frames[0].picture.size,
                        [
                            r["size"][0].as_u64().unwrap() as u32,
                            r["size"][1].as_u64().unwrap() as u32
                        ]
                    );
                    let pixels: Vec<u8> = frames[0]
                        .picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().map(|v| *v as u8))
                        .collect();
                    assert_eq!(pixels, expected, "{}", r["file"]);
                }
            }
            assert_eq!(count, r["groups"].as_u64().unwrap());
            d.reset();
        }
        let data = std::fs::read(root().join(r["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), 16 << 20).unwrap();
        for _ in 0..2 {
            let f = reader.read_frame_planes().unwrap().unwrap();
            assert_eq!(
                f.y.iter()
                    .chain(&f.cb)
                    .chain(&f.cr)
                    .copied()
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_planes().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        let f = reader.read_frame_planes().unwrap().unwrap();
        assert_eq!(
            f.y.iter()
                .chain(&f.cb)
                .chain(&f.cr)
                .copied()
                .collect::<Vec<_>>(),
            expected
        );
    }
}
#[test]
fn pending_header_reset_padding_and_compressed_budget_are_checked() {
    let m = manifest();
    let bytes = std::fs::read(root().join(m["fixtures"][0]["file"].as_str().unwrap())).unwrap();
    let obus: Vec<_> = fvid::codec::av1::Obus::new(&bytes)
        .map(Result::unwrap)
        .collect();
    let seq = packet(1, 0, 0, obus[0].payload);
    let header = packet(3, 0, 0, obus[1].payload);
    let tile = packet(4, 0, 0, obus[2].payload);
    let mut d = Decoder::new(16 << 20);
    assert!(d.decode_packet(&seq).unwrap().is_empty());
    assert!(d.decode_packet(&header).unwrap().is_empty());
    let mut padded = obus[1].payload.to_vec();
    padded.extend_from_slice(&[0, 0]);
    assert!(d
        .decode_packet(&packet(7, 0, 0, &padded))
        .unwrap()
        .is_empty());
    assert_eq!(d.decode_packet(&tile).unwrap().len(), 1);
    d.reset();
    assert!(d.decode_packet(&seq).unwrap().is_empty());
    assert!(d.decode_packet(&header).unwrap().is_empty());
    d.reset();
    assert!(d.decode_packet(&seq).unwrap().is_empty());
    assert!(d
        .decode_packet(&tile)
        .err()
        .unwrap()
        .to_string()
        .contains("tile group without frame header"));
    let mut d = Decoder::new(obus[1].payload.len() + obus[2].payload.len() - 1);
    d.decode_packet(&seq).unwrap();
    d.decode_packet(&header).unwrap();
    assert!(d
        .decode_packet(&tile)
        .err()
        .unwrap()
        .to_string()
        .contains("pending tiles exceed memory budget"));
}

#[test]
fn end_of_input_and_configuration_reject_incomplete_frames() {
    let m = manifest();
    assert_eq!(m["eof_refusals"].as_array().unwrap().len(), 4);
    for r in m["eof_refusals"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(r["file"].as_str().unwrap())).unwrap();
        let mut d = Decoder::new(16 << 20);
        assert!(d.decode_packet(&bytes).unwrap().is_empty());
        assert!(d
            .finish()
            .err()
            .unwrap()
            .to_string()
            .contains(r["error"].as_str().unwrap()));
        let data = std::fs::read(root().join(r["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), 16 << 20).unwrap();
        for _ in 0..2 {
            let error = reader
                .read_frame_planes()
                .err()
                .expect("incomplete container accepted")
                .to_string();
            assert!(error.contains(r["error"].as_str().unwrap()), "{error}");
            reader.rewind();
        }
    }
    let bytes = std::fs::read(root().join(m["fixtures"][0]["file"].as_str().unwrap())).unwrap();
    let mut configuration = vec![0x81, 0, 0, 0];
    for o in fvid::codec::av1::Obus::new(&bytes).take(2) {
        let o = o.unwrap();
        configuration.extend(packet(o.kind, o.temporal_id, o.spatial_id, o.payload));
    }
    assert!(Decoder::from_configuration(&configuration, 16 << 20)
        .err()
        .expect("coded configuration accepted")
        .to_string()
        .contains("configuration contains coded frames"));
    let mut d = Decoder::new(16 << 20);
    d.finish().unwrap();
    d.decode_packet(&bytes).unwrap();
    d.finish().unwrap();
}

#[test]
fn mp4_separate_groups_preserve_pixels_and_refuse_incomplete_end() {
    let m = manifest();
    for r in m["multitile"].as_array().unwrap() {
        let data = std::fs::read(root().join(r["mp4"].as_str().unwrap())).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            fvid::container::mp4::Limits::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let f = reader.read_frame().unwrap().unwrap();
            let p = &f.picture;
            assert_eq!(
                p.y.iter()
                    .chain(&p.cb)
                    .chain(&p.cr)
                    .map(|v| *v as u8)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(f.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(f.duration.nanoseconds().unwrap(), 20_000_000);
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0), 0);
        let f = reader.read_frame().unwrap().unwrap();
        let p = &f.picture;
        assert_eq!(
            p.y.iter()
                .chain(&p.cb)
                .chain(&p.cr)
                .map(|v| *v as u8)
                .collect::<Vec<_>>(),
            expected
        );
    }
    for r in m["eof_refusals"].as_array().unwrap() {
        let data = std::fs::read(root().join(r["mp4"].as_str().unwrap())).unwrap();
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
            fvid::container::mp4::Limits::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let error = reader
                .read_frame()
                .err()
                .expect("incomplete MP4 accepted")
                .to_string();
            assert!(error.contains(r["error"].as_str().unwrap()), "{error}");
            reader.rewind();
        }
    }
}
