//! Actual decoded mixed intra/inter chroma groups, with complete pixel/playback acceptance.
use fvid::codec::av1_decoder::Decoder;
fn pixels(p: &fvid::codec::av1_picture::Picture) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, plane) in p.planes.iter().enumerate() {
        let d = if index == 0 { 1 } else { 2 };
        for y in 0..(p.size[1] as usize).div_ceil(d) {
            for x in 0..(p.size[0] as usize).div_ceil(d) {
                let v = plane.samples[y * plane.width + x];
                if p.depth == 8 {
                    bytes.push(v as u8);
                } else {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    bytes
}
fn raw(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected raw AV1"),
    }
}
#[test]
fn owned_mixed_sub8_chroma_pixels_reset_timestamps_and_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("av1-mixed-sub8-generated.json")).unwrap())
            .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 9);
    let mut total = [0; 3];
    let mut all_inter = 0;
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let count = record["frames"].as_u64().unwrap() as usize;
        assert_eq!(count, 4);
        let frame_bytes = expected.len() / count;
        let mut decoder = Decoder::new(16 << 20);
        for replay in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), count);
            let mut groups = [0; 3];
            let mut distinct = 0;
            for (i, frame) in frames.iter().enumerate() {
                for (n, v) in groups
                    .iter_mut()
                    .zip(frame.picture.mixed_intra_chroma_groups)
                {
                    *n += v;
                }
                distinct += frame.picture.sub8_inter_chroma_groups;
                let bytes = pixels(&frame.picture);
                assert_eq!(bytes.len(), frame_bytes);
                assert_eq!(
                    bytes
                        .iter()
                        .zip(&expected[i * frame_bytes..(i + 1) * frame_bytes])
                        .position(|(a, b)| a != b),
                    None,
                    "{name} frame {i}: first differing byte"
                );
            }
            eprintln!("{name}: mixed={groups:?}, all-inter={distinct}");
            assert!(groups[0] > 0, "{name}: actual 4x4 mixed group");
            assert!(distinct > 0, "{name}: actual all-inter sub8 group");
            if record["orientation"].as_u64().unwrap() > 0 {
                assert!(
                    groups[1] > 0 && groups[2] > 0,
                    "{name}: both rectangular mixed shapes"
                );
            }
            if replay == 0 {
                for (n, v) in total.iter_mut().zip(groups) {
                    *n += v;
                }
                all_inter += distinct;
            }
            decoder.finish().unwrap();
            decoder.reset();
        }
        let container = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(container), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            for i in 0..count {
                assert_eq!(
                    raw(reader.read_frame_raw().unwrap().unwrap()),
                    expected[i * frame_bytes..(i + 1) * frame_bytes],
                    "{name}: WebM frame {i}"
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
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(40_000_000).unwrap(), 0);
        for i in 0..count {
            assert_eq!(
                raw(reader.read_frame_raw().unwrap().unwrap()),
                expected[i * frame_bytes..(i + 1) * frame_bytes],
                "{name}: seek frame {i}"
            );
        }
    }
    eprintln!("total mixed shapes={total:?} all-inter={all_inter}");
    assert!(total.iter().all(|n| *n > 0));
    assert!(all_inter > 0);
}
