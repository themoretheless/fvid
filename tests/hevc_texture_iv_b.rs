use fvid::playback::video::{
    mp4::{LayeredMp4VideoReader, Mp4VideoReader},
    native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::Path, time::Duration};

#[test]
fn texture_iv_b_lists_weights_pixels_reordering_and_repeated_seek_match_oracles() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("texture-iv-b-manifest.json")).unwrap())
            .unwrap();
    let bytes = std::fs::read(root.join("texture-iv-b-oracles.y16")).unwrap();
    let golden: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 88);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let source_offset = case["offsets"][0].as_u64().unwrap() as usize;
        let target_offset = case["offsets"][1].as_u64().unwrap() as usize;
        let source_pixels = &golden[source_offset..source_offset + 1024];
        let target_pixels = &golden[target_offset..target_offset + 1024];
        let input =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(data.clone()), Default::default())
                .unwrap();
        let timing: Vec<_> = (0..3)
            .map(|i| {
                let s = input.tracks()[0].samples.get(i).unwrap();
                (s.dts, s.pts)
            })
            .collect();
        assert_eq!(timing, [(0, 0), (1, 2), (2, 1)], "{name}: encoded order");
        if case["maximum"].as_u64() == Some(6) {
            use fvid::codec::{
                hevc_decoder::{LayeredAccessUnit, LayeredParameters},
                hevc_poc::LayeredPocState,
            };
            let mut input = fvid::container::mp4::Mp4Reader::open(
                Cursor::new(data.clone()),
                Default::default(),
            )
            .unwrap();
            let mut params =
                LayeredParameters::from_configuration(&input.tracks()[0].configuration, 64 << 20)
                    .unwrap();
            let mut state = LayeredPocState::default();
            let mut packet = Vec::new();
            for sample in 0..3 {
                input.read_packet(0, sample, &mut packet).unwrap();
                let plan =
                    LayeredAccessUnit::parse(&params, &state, &packet, 4, 1, 64 << 20).unwrap();
                if sample == 2 {
                    let target = plan
                        .pictures()
                        .iter()
                        .find(|p| p.binding.sps.layer_id == 5)
                        .unwrap();
                    assert_eq!(
                        target.slices[0].max_merge_candidates, 6,
                        "{name}: actual six-candidate header"
                    );
                }
                let (next, transition, _, _) = plan.into_parts();
                params = next;
                if let Some(t) = transition {
                    state = t.into_state();
                }
            }
        }

        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Default::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                assert_eq!(frame.presentation_time.ticks, i64::from(poc));
                assert_eq!(frame.duration.ticks, 1);
                let picture = &frame.decoded.picture;
                if poc == 1 && id == 3 {
                    assert_eq!(
                        picture.planes[0].samples(),
                        source_pixels,
                        "{name}: source pixels"
                    );
                } else if poc == 1 && id == 5 {
                    assert_eq!(
                        picture.planes[0].samples(),
                        target_pixels,
                        "{name}: target pixels"
                    );
                } else {
                    for y in 0..32 {
                        for x in 0..32 {
                            assert_eq!(
                                picture.planes[0].samples()[y * 32 + x],
                                ((x * 3 + y * 5 + 17 + if poc == 2 { 97 } else { 0 }) % 256) as u16,
                                "{name}: layer{id} POC{poc} at{x},{y}"
                            );
                        }
                    }
                }
                if poc != 1 {
                    continue;
                }
                for y in (0..32).step_by(8) {
                    for x in (0..32).step_by(8) {
                        let motion = picture.prediction_motion_at([x, y]).unwrap();
                        if id == 3 {
                            if case["intra"].as_bool().unwrap() {
                                assert_eq!(motion, [None; 2]);
                                continue;
                            }
                            let mode = case["mode"].as_str().unwrap();
                            for list in 0..2 {
                                let active = mode == "bi"
                                    || mode == "l0" && list == 0
                                    || mode == "l1" && list == 1;
                                assert_eq!(
                                    motion[list].is_some(),
                                    active,
                                    "{name}: source L{list}"
                                );
                                if let Some(v) = motion[list] {
                                    assert_eq!(v.reference, 0);
                                    assert_eq!(v.mv, [0, if list == 0 { 4 } else { -8 }]);
                                }
                            }
                        } else if id == 5 {
                            assert_eq!(
                                frame.decoded.binding.pps.pps.weighted_biprediction,
                                case["weighted"].as_bool().unwrap()
                            );
                            for list in 0..2 {
                                let v = motion[list].unwrap();
                                assert_eq!(
                                    v.reference,
                                    case["references"][list].as_u64().unwrap() as u8,
                                    "{name}: target L{list}"
                                );
                                let expected: [i64; 2] = std::array::from_fn(|axis| {
                                    case["motion"][list][axis].as_i64().unwrap()
                                });
                                assert_eq!(v.mv, expected, "{name}: target L{list}");
                            }
                            let state = picture.inter_view_disparity_at([x, y]).unwrap();
                            assert_eq!(
                                state.is_some(),
                                case["label"].as_str() == Some("IV"),
                                "{name}: IV flag"
                            );
                            if let Some(state) = state {
                                assert_eq!(state.vector.reference_view, 1);
                                assert_eq!(state.vector.mv, [0, 0]);
                                assert_eq!(state.refined_mv, [0, 0]);
                            }
                        }
                    }
                }
            }
            assert_eq!(
                seen,
                [
                    (0, 0),
                    (3, 0),
                    (5, 0),
                    (0, 1),
                    (3, 1),
                    (5, 1),
                    (0, 2),
                    (3, 2),
                    (5, 2)
                ]
            );
            reader.rewind();
        }
        let pixels = |poc: i64| -> Vec<u8> {
            if poc == 1 {
                return target_pixels.iter().map(|&v| v as u8).collect();
            }
            (0..32)
                .flat_map(|y| {
                    (0..32).map(move |x| {
                        ((3 * x + 5 * y + 17 + if poc == 2 { 97 } else { 0 }) % 256) as u8
                    })
                })
                .collect()
        };
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Default::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = common
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: common {e}"))
            {
                assert_eq!(common.selected_layer_id(), Some(5));
                let poc = frame.presentation_time.ticks;
                seen.push(poc);
                let packed = frame.packed.unwrap();
                assert_eq!(
                    &packed.frame.data[..1024],
                    pixels(poc),
                    "{name}: common POC{poc}"
                );
                assert!(packed.frame.data[1024..].iter().all(|&v| v == 128));
            }
            assert_eq!(seen, [0, 1, 2]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for (ms, poc) in [(40, 1), (80, 2), (0, 0), (40, 1), (40, 1)] {
            let raw = native
                .seek_raw(Duration::from_millis(ms))
                .unwrap_or_else(|e| panic!("{name}: seek{ms}: {e}"))
                .unwrap();
            let RawFrame::Planar(packed) = raw else {
                panic!("{name}: expected planar")
            };
            assert_eq!(
                &packed.frame.data[..1024],
                pixels(poc),
                "{name}: seek POC{poc}"
            );
        }
    }
}
