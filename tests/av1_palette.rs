use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn palette_pixels_reset_webm_rewind_seek_match_owned_oracle() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("av1-palette-generated.json")).unwrap())
            .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 168);
    for depth in [8, 10, 12] {
        assert_eq!(records.iter().filter(|r| r["depth"] == depth).count(), 56);
    }
    let mut coverage = [[[0u32; 7]; 2]; 3];
    for r in records {
        assert_eq!(r["oracles"], serde_json::json!(["libaom", "dav1d"]));
        if !r["mirror"].as_bool().unwrap() {
            assert!(r["source_exact"].as_bool().unwrap());
        }
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let size = [
            r["size"][0].as_u64().unwrap() as u32,
            r["size"][1].as_u64().unwrap() as u32,
        ];
        let depth = r["depth"].as_u64().unwrap() as u8;
        let mut d = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = d
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            assert!(frames[0].show);
            let p = &frames[0].picture;
            assert_eq!(p.size, size);
            assert_eq!(p.depth, depth);
            let stats = p.palette_counts;
            let depth_index = match depth {
                8 => 0,
                10 => 1,
                12 => 2,
                _ => unreachable!(),
            };
            for plane in 0..2 {
                for size in 0..7 {
                    coverage[depth_index][plane][size] += stats[plane][size];
                }
            }
            assert!(
                stats[0][r["colors"].as_u64().unwrap() as usize - 2] > 0,
                "{name}: actual Y palette sizes {stats:?}"
            );
            if r["chroma"].as_bool().unwrap() {
                assert!(
                    stats[1].iter().sum::<u32>() > 0,
                    "{name}: UV palette absent {stats:?}"
                );
            }
            assert!(
                p.palette_cache_hits[0] > 0,
                "{name}: no Y cached colors exercised"
            );
            if r["chroma"].as_bool().unwrap() {
                assert!(
                    p.palette_cache_hits[1] > 0,
                    "{name}: no UV cached colors exercised"
                );
            }
            let mut actual = Vec::new();
            for (index, plane) in p.planes.iter().enumerate() {
                let w = (p.size[0] as usize).div_ceil(if index == 0 { 1 } else { 2 });
                let h = (p.size[1] as usize).div_ceil(if index == 0 { 1 } else { 2 });
                for row in 0..h {
                    for col in 0..w {
                        let v = plane.samples[row * plane.width + col];
                        if depth == 8 {
                            actual.push(v as u8);
                        } else {
                            actual.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(actual, expected, "{name}");
            d.finish().unwrap();
            d.reset();
        }
        let data = std::fs::read(root().join(r["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), 16 << 20).unwrap();
        for _ in 0..2 {
            let f = reader.read_frame_raw().unwrap().unwrap();
            assert_eq!(raw_bytes(f), expected, "{name}: raw WebM");
            assert_eq!(reader.dimensions(), size.map(|v| v as usize));
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
        let f = reader.read_frame_raw().unwrap().unwrap();
        assert_eq!(raw_bytes(f), expected, "{name}: seek");
    }
    for (depth, groups) in coverage.iter().enumerate() {
        for (plane, sizes) in groups.iter().enumerate() {
            for (index, count) in sizes.iter().enumerate() {
                assert!(
                    *count > 0,
                    "missing actual palette size {} in depth index {depth}, plane {plane}",
                    index + 2
                );
            }
        }
    }
}
fn raw_bytes(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected AV1 raw frame representation"),
    }
}
