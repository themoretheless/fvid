use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-mixed-inter-generated.json")).unwrap())
        .unwrap()
}
#[test]
fn mixed_inter_lossless_prediction_residuals_and_reference_roles_match_oracle() {
    let m = manifest();
    let fixtures = m["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 672);
    for reference in 1..=7 {
        assert_eq!(
            fixtures
                .iter()
                .filter(|r| r["logical_reference"].as_u64() == Some(reference))
                .count(),
            96
        );
    }
    for r in fixtures {
        let name = r["file"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        assert_eq!(expected.len(), 3072);
        assert!(expected.iter().any(|&p| p != 128));
        let mut sequence = None;
        let mut refs: [Option<Header>; 8] = std::array::from_fn(|_| None);
        let mut inter = 0;
        for obu in Obus::new(&data) {
            let obu = obu.unwrap();
            if obu.kind == 1 {
                sequence = Some(Sequence::parse(obu.payload).unwrap());
            }
            if obu.kind == 6 {
                let h = Header::parse(
                    sequence.as_ref().unwrap(),
                    obu.payload,
                    0,
                    0,
                    &std::array::from_fn(|i| refs[i].as_ref()),
                )
                .unwrap();
                assert!(h.lossless[0]);
                assert!(!h.lossless[1]);
                if h.frame_type == 1 {
                    inter += 1;
                    assert!(h.show);
                    assert_eq!(h.tx_mode, if r["selected"] == true { 2 } else { 1 });
                    assert_eq!(h.disable_cdf_update, r["adaptive"] == false);
                    assert!(h.segmentation_update_map);
                    assert!(!h.segmentation_temporal_update);
                    let expected_refs:Vec<usize>=r["physical_references"].as_array().unwrap().iter().map(|v|v.as_u64().unwrap() as usize).collect();
                    assert_eq!(h.references.as_slice(),expected_refs);
                    assert_eq!(h.refresh_flags,128);
                }
                for i in 0..8 {
                    if h.refresh_flags & (1 << i) != 0 {
                        refs[i] = Some(h.clone());
                    }
                }
            }
        }
        assert_eq!(inter, 2);
        let mut d = Decoder::new(16 << 20);
        for _ in 0..2 {
            let frames = d
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), 4);
            assert!(!frames[0].show);
            assert!(!frames[1].show);
            assert!(frames[2].show && frames[3].show);
            for (i, f) in frames.iter().enumerate() {
                let map = r["maps"][i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>();
                assert_eq!(f.picture.segment_ids, map, "{name} {i}");
            }
            let pixels = frames
                .iter()
                .filter(|f| f.show)
                .flat_map(|f| {
                    f.picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().copied())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                pixels,
                expected.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(),
                "{name}"
            );
            d.reset();
        }
    }
}
#[test]
fn mixed_inter_webm_pixels_timestamps_rewind_and_seek() {
    for r in manifest()["fixtures"].as_array().unwrap() {
        let name = r["webm"].as_str().unwrap();
        let data = std::fs::read(root().join(name)).unwrap();
        let expected = std::fs::read(root().join(r["reference"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(Cursor::new(&data), 16 << 20).unwrap();
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
                    expected[i * 1536..(i + 1) * 1536],
                    "{name} {i}"
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
        assert!(reader.read_frame_planes().unwrap().is_some());
    }
}
