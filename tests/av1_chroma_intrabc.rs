use fvid::codec::av1_decoder::Decoder;
// Owned native AV1 pixel and playback acceptance; external codecs run only at generation.
fn qualify(control: bool) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join(if control {
            "av1-chroma-intrabc-control-generated.json"
        } else {
            "av1-chroma-intrabc-generated.json"
        }))
        .unwrap(),
    )
    .unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 24);
    let mut groups = [[0usize; 2]; 3];
    let mut parity_groups = [[[0u32; 4]; 2]; 3];
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
        let sub = match record["layout"].as_str().unwrap() {
            "422" => [true, false],
            "444" => [false, false],
            _ => panic!("unexpected chroma layout"),
        };
        assert_eq!(sequence.color.subsampling, sub);
        assert_eq!(
            sequence.color.depth as u64,
            record["depth"].as_u64().unwrap()
        );
        let group = (sequence.color.depth as usize - 8) / 2;
        let layout = usize::from(!sub[0]);
        groups[group][layout] += 1;
        let frame = obus.iter().find(|o| o.kind == 6).unwrap();
        let header = fvid::codec::av1_frame::Header::parse_intra(
            &sequence,
            frame.payload,
            frame.temporal_id,
            frame.spatial_id,
        )
        .unwrap();
        assert_eq!(
            header.intrabc, !control,
            "{name}: actual intra block copy permission"
        );
        assert_eq!(
            header.lossless.iter().all(|v| *v),
            record["lossless"].as_bool().unwrap()
        );
        assert_eq!(header.tiles.columns.len(), 3, "{name}: two tiles");
        assert_eq!(sequence.superblock128, record["sb"] == 128);
        let mut decoder = Decoder::new(32 << 20);
        for replay in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 1);
            let picture = &frames[0].picture;
            assert_eq!(
                picture.size,
                [
                    record["size"][0].as_u64().unwrap() as u32,
                    record["size"][1].as_u64().unwrap() as u32
                ]
            );
            assert_eq!(picture.subsampling, sub);
            assert_eq!(picture.depth, sequence.color.depth);
            assert_eq!(
                picture.intrabc_blocks > 0,
                !control,
                "{name}: actual copied blocks"
            );
            if !control {
                assert!(
                    picture.intrabc_sub8_blocks > 0,
                    "{name}: actual sub-8x8 copies"
                );
                if replay == 0 {
                    for (total, count) in parity_groups[group][layout]
                        .iter_mut()
                        .zip(picture.intrabc_displacement_parities)
                    {
                        *total += count;
                    }
                }
                let phases = picture.intrabc_phases;
                if sub[0] {
                    assert!(
                        phases[0] > 0 && phases[1] > 0,
                        "{name}: horizontal chroma half phases"
                    );
                    assert_eq!(
                        phases[2..],
                        [0; 2],
                        "{name}: full-height chroma has no vertical half phase"
                    );
                } else {
                    assert_eq!(
                        phases,
                        [picture.intrabc_blocks, 0, 0, 0],
                        "{name}: full-resolution chroma has integral copies"
                    );
                }
                assert!(
                    picture.intrabc_residual_blocks.iter().all(|v| *v > 0),
                    "{name}: nonzero Y/U/V copy residuals"
                );
            }
            if control {
                assert_eq!(picture.intrabc_sub8_blocks, 0);
                assert_eq!(picture.intrabc_phases, [0; 4]);
                assert_eq!(picture.intrabc_displacement_parities, [0; 4]);
                assert_eq!(picture.intrabc_residual_blocks, [0; 3]);
            }
            eprintln!(
                "{name}: intrabc={} residuals={:?} phases={:?} parity={:?} sub8={}",
                picture.intrabc_blocks,
                picture.intrabc_residual_blocks,
                picture.intrabc_phases,
                picture.intrabc_displacement_parities,
                picture.intrabc_sub8_blocks
            );
            let mut pixels = Vec::new();
            for (index, plane) in picture.planes.iter().enumerate() {
                let [sx, sy] = if index == 0 {
                    [1; 2]
                } else {
                    sub.map(|s| 1 << usize::from(s))
                };
                for y in 0..(picture.size[1] as usize).div_ceil(sy) {
                    for x in 0..(picture.size[0] as usize).div_ceil(sx) {
                        let value = plane.samples[y * plane.width + x];
                        if picture.depth == 8 {
                            pixels.push(value as u8);
                        } else {
                            pixels.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
            let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
            assert_eq!(pixels.len(), expected.len(), "{name}: pixel extent");
            assert_eq!(
                pixels.iter().zip(&expected).position(|(a, b)| a != b),
                None,
                "{name}: first differing byte"
            );
            decoder.finish().unwrap();
            decoder.reset();
        }
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let webm = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(webm), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            assert_eq!(
                raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
                expected,
                "{name}: WebM replay"
            );
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(10_000_000).unwrap(), 0);
        assert_eq!(
            raw_bytes(reader.read_frame_raw().unwrap().unwrap()),
            expected,
            "{name}: seek"
        );
        assert_eq!(
            reader.frame_interval(),
            Some((0, 20_000_000, 1_000_000_000))
        );
    }
    assert!(groups.iter().flatten().all(|n| *n == 4));
    if !control {
        assert!(
            parity_groups.iter().flatten().flatten().all(|n| *n > 0),
            "all displacement parities in each layout/depth group: {parity_groups:?}"
        );
    }
}

#[test]
fn owned_chroma_intrabc_tiles_odd_displacements_and_small_blocks() {
    qualify(false);
}
#[test]
fn owned_chroma_intrabc_disabled_control_pixels_reset_and_seek() {
    qualify(true);
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
