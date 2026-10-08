//! Owned temporal SVC operating-point acceptance; no external codec at test time.
use fvid::codec::{
    av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_picture::Picture,
    av1_sequence::Sequence,
};
fn pixels(p: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, plane) in p.planes.iter().enumerate() {
        let d = if i == 0 { 1 } else { 2 };
        for y in 0..(p.size[1] as usize).div_ceil(d) {
            out.extend(
                plane.samples[y * plane.width..y * plane.width + (p.size[0] as usize).div_ceil(d)]
                    .iter()
                    .map(|v| *v as u8),
            );
        }
    }
    out
}
fn raw(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        _ => panic!("expected 8-bit SVC frame"),
    }
}
#[test]
fn owned_temporal_operating_points_pixels_selection_configuration_reset_and_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-temporal-operating-points-generated.json")).unwrap(),
    )
    .unwrap();
    let data = std::fs::read(root.join(manifest["file"].as_str().unwrap())).unwrap();
    let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
    let seq = obus.iter().find(|o| o.kind == 1).unwrap();
    let sequence = Sequence::parse(seq.payload).unwrap();
    assert_eq!(
        sequence
            .operating_points
            .iter()
            .map(|p| p.idc)
            .collect::<Vec<_>>(),
        vec![0x107, 0x103, 0x101]
    );
    assert_eq!(
        obus.iter()
            .filter(|o| o.kind == 6)
            .map(|o| {
                assert!(o.has_extension);
                o.temporal_id
            })
            .collect::<Vec<_>>(),
        vec![0, 2, 1, 2, 0, 2, 1, 2]
    );
    // Split access units at actual delimiters, including the sequence in the first.
    let mut starts = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let start = at;
        let header = data[at];
        at += 1;
        if header & 4 != 0 {
            at += 1;
        }
        let mut len = 0;
        let mut shift = 0;
        loop {
            let b = data[at];
            at += 1;
            len |= usize::from(b & 127) << shift;
            shift += 7;
            if b & 128 == 0 {
                break;
            }
        }
        at += len;
        if header >> 3 == 2 {
            starts.push(start);
        }
    }
    assert_eq!(starts.len(), 8);
    starts.push(data.len());
    let seq_end = seq.payload.as_ptr() as usize - data.as_ptr() as usize + seq.payload.len();
    let mut config = vec![0x81, 0, 0, 0];
    // The first delimiter is two bytes and the codec config stores the sequence OBU.
    assert_eq!(data[..2], [0x12, 0]);
    config.extend_from_slice(&data[2..seq_end]);
    for point in manifest["points"].as_array().unwrap() {
        let index = point["point"].as_u64().unwrap() as usize;
        let golden = std::fs::read(root.join(point["reference"].as_str().unwrap())).unwrap();
        let mut saved: [Option<Header>; 8] = std::array::from_fn(|_| None);
        let mut invalidated = 0;
        let mut tested_active_reference = false;
        let idc = point["idc"].as_u64().unwrap() as u16;
        for obu in obus
            .iter()
            .filter(|o| o.kind == 6 && idc & (1 << o.temporal_id) != 0)
        {
            let header = Header::parse(
                &sequence,
                obu.payload,
                obu.temporal_id,
                obu.spatial_id,
                &saved.each_ref().map(|h| h.as_ref()),
            )
            .unwrap();
            if index == 0
                && !tested_active_reference
                && header.frame_type == 1
                && header.error_resilient
            {
                let mut missing_refs = saved.clone();
                missing_refs[header.references[0]]
                    .as_mut()
                    .unwrap()
                    .order_hint ^= 1;
                let error = Header::parse(
                    &sequence,
                    obu.payload,
                    obu.temporal_id,
                    obu.spatial_id,
                    &missing_refs.each_ref().map(|h| h.as_ref()),
                )
                .err()
                .unwrap();
                assert!(error.to_string().contains("references missing picture"));
                tested_active_reference = true;
            }
            invalidated += header.invalidated_references.count_ones();
            if header.frame_type == 1 {
                assert!(
                    header
                        .references
                        .iter()
                        .all(|r| header.invalidated_references & (1 << r) == 0)
                );
            }
            for i in 0..8 {
                if header.invalidated_references & (1 << i) != 0 {
                    saved[i] = None;
                }
                if header.refresh_flags & (1 << i) != 0 {
                    saved[i] = Some(header.clone());
                }
            }
        }
        if index == 0 {
            assert!(tested_active_reference);
        }
        if index > 0 {
            assert!(
                invalidated > 0,
                "point {index}: actual inactive reference invalidation"
            );
        }
        let count = point["indices"].as_array().unwrap().len();
        assert_eq!(golden.len(), count * 64 * 48 * 3 / 2);
        for seeded in [false, true] {
            let mut decoder = if seeded {
                Decoder::from_configuration_with_operating_point(&config, 16 << 20, index).unwrap()
            } else {
                Decoder::with_operating_point(16 << 20, index).unwrap()
            };
            for packetized in [false, true] {
                let mut actual = Vec::new();
                if packetized {
                    for pair in starts.windows(2) {
                        for frame in decoder.decode_packet(&data[pair[0]..pair[1]]).unwrap() {
                            assert!(frame.show);
                            actual.extend(pixels(&frame.picture));
                        }
                    }
                } else {
                    for frame in decoder.decode_packet(&data).unwrap() {
                        assert!(frame.show);
                        actual.extend(pixels(&frame.picture));
                    }
                }
                assert_eq!(
                    actual, golden,
                    "point {index} config={seeded} packetized={packetized}"
                );
                decoder.finish().unwrap();
                decoder.reset();
            }
        }
    }
    let mut default = Decoder::new(16 << 20);
    assert_eq!(default.decode_packet(&data).unwrap().len(), 8);
    assert!(Decoder::with_operating_point(16 << 20, 32).is_err());
    assert!(Decoder::from_configuration_with_operating_point(&config, 16 << 20, 3).is_err());
    let mut absent = Decoder::with_operating_point(16 << 20, 3).unwrap();
    assert!(
        absent
            .decode_packet(&data)
            .err()
            .unwrap()
            .to_string()
            .contains("index not present")
    );
    absent.reset();
    assert!(absent.decode_packet(&data).is_err());
    let expected =
        std::fs::read(root.join(manifest["points"][0]["reference"].as_str().unwrap())).unwrap();
    let container = std::fs::read(root.join(manifest["webm"].as_str().unwrap())).unwrap();
    let mut reader =
        fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(container), 16 << 20)
            .unwrap();
    for _ in 0..2 {
        for (i, frame) in expected.chunks_exact(64 * 48 * 3 / 2).enumerate() {
            assert_eq!(raw(reader.read_frame_raw().unwrap().unwrap()), frame);
            assert_eq!(
                reader.frame_interval(),
                Some((
                    i as u128 * 20_000_000,
                    (i as u128 + 1) * 20_000_000,
                    1_000_000_000
                ))
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind();
    }
    assert_eq!(reader.seek_to_sync(100_000_000).unwrap(), 0);
    for frame in expected.chunks_exact(64 * 48 * 3 / 2) {
        assert_eq!(raw(reader.read_frame_raw().unwrap().unwrap()), frame);
    }
}

#[test]
fn excluded_extended_obus_do_not_parse_payload_or_mutate_metadata() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let data = std::fs::read(root.join("av1-temporal-operating-points.obu")).unwrap();
    let seq = Obus::new(&data)
        .map(Result::unwrap)
        .find(|o| o.kind == 1)
        .unwrap();
    let end = seq.payload.as_ptr() as usize - data.as_ptr() as usize + seq.payload.len();
    let mut decoder = Decoder::new(16 << 20);
    assert!(decoder.decode_packet(&data[..end]).unwrap().is_empty());
    let before = decoder.hdr();
    // Temporal 7/spatial 3 is outside all fixture operating points. Empty
    // frame/header/tile/tile-list payloads must be dropped before their parsers.
    for extension in [0xe0, 0x18, 0xf8] {
        for kind in [3, 4, 6, 7, 8] {
            assert!(
                decoder
                    .decode_packet(&[(kind << 3) | 6, extension, 0])
                    .unwrap()
                    .is_empty()
            );
        }
    }
    // Valid excluded HDR CLL must not alter the selected stream's metadata.
    assert!(
        decoder
            .decode_packet(&[0x2e, 0xf8, 6, 1, 3, 232, 1, 144, 128])
            .unwrap()
            .is_empty()
    );
    assert_eq!(decoder.hdr(), before);
    let frames = decoder.decode_packet(&data[end..]).unwrap();
    assert_eq!(frames.len(), 8);
    decoder.finish().unwrap();
    assert!(
        decoder
            .decode_packet(&[0x2a, 6, 1, 3, 232, 1, 144, 128])
            .unwrap()
            .is_empty()
    );
    assert_ne!(
        decoder.hdr(),
        before,
        "the same unextended HDR payload is valid and global"
    );
    // Included empty spatial frames must reach payload validation. Actual spatial
    // reconstruction is qualified by the owned two-layer acceptance suite.
    let mut included = Decoder::new(16 << 20);
    let mut spatial_sequence = data[..end].to_vec();
    let start = seq.payload.as_ptr() as usize - data.as_ptr() as usize;
    spatial_sequence[start + 1] |= 2; // authored op0 idc 0x107 -> 0x307, include spatial 1
    included.decode_packet(&spatial_sequence).unwrap();
    let error = included
        .decode_packet(&[0x36, 8, 0])
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("not implemented"), "{error}");
    assert!(error.contains("truncated"), "{error}");
}
