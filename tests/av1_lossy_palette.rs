use fvid::codec::av1_decoder::Decoder;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn lossy_palette_pixels_reset_webm_rewind_seek_match_owned_oracle() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("av1-lossy-palette-generated.json")).unwrap(),
    )
    .unwrap();
    let mut records: Vec<_> = m["fixtures"].as_array().unwrap().iter().collect();
    records.sort_by_key(|r| std::cmp::Reverse(r["quality"].as_u64().unwrap()));
    assert_eq!(records.len(), 339);
    for depth in [8, 10, 12] {
        assert_eq!(records.iter().filter(|r| r["depth"] == depth).count(), 113);
    }
    let mut coverage = [[[0u32; 7]; 2]; 3];
    let mut residuals = [[0u32; 2]; 3];
    let mut filters = [0u32; 3];
    for r in records {
        assert_eq!(r["oracles"], serde_json::json!(["libaom", "dav1d"]));
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let size = [
            r["size"][0].as_u64().unwrap() as u32,
            r["size"][1].as_u64().unwrap() as u32,
        ];
        let depth = r["depth"].as_u64().unwrap() as u8;
        let obus: Vec<_> = fvid::codec::av1::Obus::new(&bytes)
            .map(Result::unwrap)
            .collect();
        let seq = fvid::codec::av1_sequence::Sequence::parse(
            obus.iter().find(|o| o.kind == 1).unwrap().payload,
        )
        .unwrap();
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = fvid::codec::av1_frame::Header::parse_intra(
            &seq,
            frame.payload,
            frame.temporal_id,
            frame.spatial_id,
        )
        .unwrap();
        assert!(
            header.quant.base > 0 && header.lossless.iter().all(|v| !*v),
            "{name}: not lossy coded"
        );
        let index = match depth {
            8 => 0,
            10 => 1,
            12 => 2,
            _ => unreachable!(),
        };
        filters[index] += u32::from(header.filter.levels.iter().any(|v| *v > 0));
        if r["quality"] == 48 {
            assert!(
                header.filter.levels.iter().any(|v| *v > 0),
                "{name}: filtered fixture has zero levels"
            );
        }
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
            if r["quality"] == 48 {
                assert!(
                    p.palette_residual_blocks[0] > 0,
                    "{name}: no palette residuals in filtered fixture"
                );
            }
            for plane in 0..2 {
                residuals[index][plane] += p.palette_residual_blocks[plane];
            }
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
                stats[0].iter().sum::<u32>() > 0,
                "{name}: no coded Y palette {stats:?}"
            );
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
    for depth in 0..3 {
        assert!(
            filters[depth] > 0,
            "no active loop filtering in depth {depth}"
        );
        for plane in 0..2 {
            assert!(
                residuals[depth][plane] > 0,
                "no palette residuals in depth {depth}, plane {plane}"
            );
            for size in 0..7 {
                assert!(
                    coverage[depth][plane][size] > 0,
                    "no coded palette size {} in depth {depth}, plane {plane}",
                    size + 2
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
