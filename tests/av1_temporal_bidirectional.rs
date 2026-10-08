use fvid::codec::av1_decoder::Decoder;
// Owned acceptance for temporal reference motion fields and playback.
#[test]
fn owned_temporal_bidirectional_pixels_reset_timestamps_and_seek() {
    check();
}
fn check() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-temporal-bidirectional-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 6);
    let mut active_inter = 0;
    let mut future_temporal = 0;
    let mut accepted = 0;
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = fvid::codec::av1::Obus::new(&data)
            .map(Result::unwrap)
            .collect();
        let sequence = fvid::codec::av1_sequence::Sequence::parse(
            obus.iter().find(|o| o.kind == 1).unwrap().payload,
        )
        .unwrap();
        let mut references: [Option<fvid::codec::av1_frame::Header>; 8] =
            std::array::from_fn(|_| None);
        let mut kinds = Vec::new();
        for frame in obus.iter().filter(|o| o.kind == 6) {
            let header = fvid::codec::av1_frame::Header::parse(
                &sequence,
                frame.payload,
                frame.temporal_id,
                frame.spatial_id,
                &std::array::from_fn(|i| references[i].as_ref()),
            )
            .unwrap();
            assert_eq!(
                header.superres_denom,
                record["denominator"].as_u64().unwrap() as u8,
                "{name}: actual super-resolution"
            );
            assert_eq!(header.upscaled_width, 192);
            assert!(header.quant.matrix.is_none(), "{name}: isolate restoration");
            eprintln!(
                "{name}: type {} restoration {:?}",
                header.frame_type, header.restoration_types
            );
            if header.frame_type == 1 && header.reference_mvs {
                active_inter += 1;
                for &slot in &header.references {
                    if let Some(r) = &references[slot] {
                        let diff = r.order_hint.wrapping_sub(header.order_hint) as i32;
                        let shift = 32 - sequence.order_hint_bits;
                        if (diff << shift) >> shift > 0 {
                            future_temporal += 1;
                        }
                    }
                }
            }
            kinds.push(header.frame_type);
            for i in 0..8 {
                if header.refresh_flags & (1 << i) != 0 {
                    references[i] = Some(header.clone());
                }
            }
        }
        assert_eq!(kinds[0], 0);
        assert!(kinds.iter().filter(|&&k| k == 1).count() >= 11);
        accepted += 1;
        let mut decoder = Decoder::new(16 << 20);
        for _ in 0..2 {
            let mut frames = Vec::new();
            for (packet_index, packet) in packets(&data).into_iter().enumerate() {
                frames.extend(
                    decoder
                        .decode_packet(packet)
                        .unwrap_or_else(|e| panic!("{name}: OBU {packet_index}: {e}")),
                );
            }
            let frames: Vec<_> = frames.into_iter().filter(|f| f.show).collect();
            assert_eq!(frames.len(), 12);
            assert!(
                frames.iter().skip(1).any(|f| f
                    .picture
                    .saved_motion
                    .iter()
                    .any(|m| m.reference > 0 && m.mv != [0; 2])),
                "{name}: saved nonzero reference motion required"
            );
            for f in &frames {
                assert_eq!(
                    f.picture.saved_motion.len(),
                    f.picture.segment_grid[0] * f.picture.segment_grid[1] / 4
                );
            }
            let mut pixels = Vec::new();
            for frame in &frames {
                let picture = &frame.picture;
                for (index, plane) in picture.planes.iter().enumerate() {
                    let divisor = if index == 0 { 1 } else { 2 };
                    for y in 0..(picture.size[1] as usize).div_ceil(divisor) {
                        for x in 0..(picture.size[0] as usize).div_ceil(divisor) {
                            let value = plane.samples[y * plane.width + x];
                            if picture.depth == 8 {
                                pixels.push(value as u8);
                            } else {
                                pixels.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                }
            }
            let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
            let mismatch = pixels.iter().zip(&expected).position(|(a, b)| a != b);
            assert_eq!(pixels.len(), expected.len(), "{name}: pixel extent");
            assert_eq!(mismatch, None, "{name}: first differing byte");
            decoder.finish().unwrap();
            decoder.reset();
        }
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let intervals: Vec<_> = obus
            .iter()
            .filter(|o| matches!(o.kind, 3 | 6))
            .enumerate()
            .filter(|(_, o)| o.kind == 3 || o.payload[0] & 16 != 0)
            .map(|(i, _)| i as u128 * 20_000_000)
            .collect();
        assert_eq!(intervals.len(), 12);
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            for (i, expected_frame) in expected.chunks_exact(expected.len() / 12).enumerate() {
                assert_eq!(
                    raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                    expected_frame,
                    "{name}: WebM {i}"
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        intervals[i],
                        intervals
                            .get(i + 1)
                            .copied()
                            .unwrap_or(intervals[i] + 20_000_000),
                        1_000_000_000
                    )),
                    "{name}: presentation {i}"
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(100_000_000).unwrap(), 0);
        for expected_frame in expected.chunks_exact(expected.len() / 12) {
            assert_eq!(
                raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                expected_frame,
                "{name}: seek replay"
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
    assert_eq!(accepted, 6);
    assert_eq!(active_inter, 69, "actual temporal motion frames");
    assert_eq!(
        future_temporal, 78,
        "actual future-reference temporal slots"
    );
    eprintln!("temporal={active_inter} future_reference_slots={future_temporal}");
}

fn raw_bytes(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected AV1 raw representation"),
    }
}

fn packets(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let start = at;
        let extension = data[at] & 4 != 0;
        at += 1;
        if extension {
            at += 1;
        }
        let mut length = 0;
        let mut shift = 0;
        loop {
            let v = data[at];
            at += 1;
            length |= ((v & 127) as usize) << shift;
            shift += 7;
            if v & 128 == 0 {
                break;
            }
        }
        at += length;
        out.push(&data[start..at]);
    }
    out
}
