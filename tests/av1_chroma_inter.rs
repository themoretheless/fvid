//! Owned full-chroma moving inter sequences, reference scaling and offline replay.
use fvid::codec::{av1_decoder::Decoder, av1_picture::Picture};

fn pixels(p: &Picture) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (plane_index, plane) in p.planes.iter().enumerate() {
        let [sx, sy] = if plane_index == 0 {
            [1; 2]
        } else {
            p.subsampling.map(|s| 1 << usize::from(s))
        };
        for y in 0..(p.size[1] as usize).div_ceil(sy) {
            for x in 0..(p.size[0] as usize).div_ceil(sx) {
                let value = plane.samples[y * plane.width + x];
                if p.depth == 8 {
                    bytes.push(value as u8);
                } else {
                    bytes.extend_from_slice(&value.to_le_bytes());
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
        _ => panic!("unexpected AV1 output"),
    }
}
fn stats(p: &Picture) -> [u32; 10] {
    let s = &p.inter_prediction;
    [
        s.single_reference_blocks,
        s.average_compound_blocks,
        s.distance_compound_blocks,
        s.wedge_compound_blocks,
        s.difference_compound_blocks,
        s.obmc_blocks,
        s.interintra_blocks,
        s.local_warp_blocks,
        s.global_warp_blocks,
        s.scaled_reference_blocks,
    ]
}
#[test]
fn owned_full_chroma_inter_pixels_tools_scaled_references_reset_and_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-chroma-inter-generated.json")).unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 24);
    let mut groups = [[[0u32; 10]; 2]; 3];
    let mut group_count = [[0; 2]; 3];
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let depth = record["depth"].as_u64().unwrap() as u8;
        let sub = match record["layout"].as_str().unwrap() {
            "422" => [true, false],
            "444" => [false, false],
            _ => panic!("unexpected layout"),
        };
        let group = (depth as usize - 8) / 2;
        let layout = usize::from(!sub[0]);
        group_count[group][layout] += 1;
        let sizes: Vec<[u32; 2]> = record["sizes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| [s[0].as_u64().unwrap() as u32, s[1].as_u64().unwrap() as u32])
            .collect();
        assert_eq!(record["frames"], 8);
        assert_eq!(sizes.len(), 8);
        let mut offset = 0;
        let mut frame_pixels = Vec::new();
        for [w, h] in &sizes {
            let len = (*w as usize * *h as usize
                + 2 * (*w as usize).div_ceil(1 << usize::from(sub[0])) * *h as usize)
                * if depth == 8 { 1 } else { 2 };
            frame_pixels.push(&expected[offset..offset + len]);
            offset += len;
        }
        assert_eq!(offset, expected.len());
        let mut decoder = Decoder::new(64 << 20);
        for replay in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), sizes.len());
            let mut totals = [0; 10];
            for (i, frame) in frames.iter().enumerate() {
                assert!(frame.show);
                let p = &frame.picture;
                assert_eq!(p.size, sizes[i], "{name}: frame {i} geometry");
                assert_eq!(p.depth, depth);
                assert_eq!(p.subsampling, sub);
                let actual = pixels(p);
                assert_eq!(actual.len(), frame_pixels[i].len());
                assert_eq!(
                    actual.iter().zip(frame_pixels[i]).position(|(a, b)| a != b),
                    None,
                    "{name}: frame {i} first differing byte"
                );
                if replay == 0 {
                    for (total, value) in totals.iter_mut().zip(stats(p)) {
                        *total += value;
                    }
                }
            }
            if replay == 0 {
                eprintln!("{name}: prediction {totals:?}");
                for (total, value) in groups[group][layout].iter_mut().zip(totals) {
                    *total += value;
                }
                assert!(totals[0] > 0, "{name}: actual inter blocks");
                if record["resized"].as_bool().unwrap() {
                    assert!(totals[9] > 0, "{name}: actual scaled reference blocks");
                } else {
                    assert_eq!(totals[9], 0, "{name}: fixed reference sizes");
                }
            }
            decoder.finish().unwrap();
            decoder.reset();
        }
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            for (i, expected_frame) in frame_pixels.iter().enumerate() {
                assert_eq!(
                    raw(reader.read_frame_raw().unwrap().unwrap()),
                    *expected_frame,
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
        assert_eq!(reader.seek_to_sync(100_000_000).unwrap(), 0);
        for (i, expected_frame) in frame_pixels.iter().enumerate() {
            assert_eq!(
                raw(reader.read_frame_raw().unwrap().unwrap()),
                *expected_frame,
                "{name}: seek frame {i}"
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
    }
    eprintln!("per-depth/layout prediction totals: {groups:?}");
    assert!(group_count.iter().flatten().all(|n| *n == 4));
    for group in groups.iter().flatten() {
        assert!(
            group.iter().all(|n| *n > 0),
            "every selected inter tool in each layout/depth group: {group:?}"
        );
    }
}
