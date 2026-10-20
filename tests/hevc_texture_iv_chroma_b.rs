use fvid::playback::video::{
    mp4::{LayeredMp4VideoReader, Mp4VideoReader},
    native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::Path, time::Duration};

fn raw_plane(layer: u8, poc: i32, component: usize) -> Vec<u16> {
    let future = poc == 2;
    if component == 0 {
        return (0..32)
            .flat_map(|y| {
                (0..32)
                    .map(move |x| ((3 * x + 5 * y + 17 + if future { 97 } else { 0 }) % 256) as u16)
            })
            .collect();
    }
    let view = match layer {
        0 => 0,
        3 => 1,
        5 => 2,
        _ => unreachable!(),
    };
    let (sx, sy, bias, offset, delta) = if component == 1 {
        (13, 7, 29, [0, 19, 71][view], 97)
    } else {
        (17, 11, 83, [0, 37, 113][view], 194)
    };
    (0..16)
        .flat_map(|y| {
            (0..16).map(move |x| {
                ((sx * x + sy * y + bias + offset + if future { delta } else { 0 }) % 256) as u16
            })
        })
        .collect()
}

#[test]
fn colored_texture_iv_b_matches_all_planes_and_full_software_packing() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-iv-chroma-b-manifest.json")).unwrap(),
    )
    .unwrap();
    let bytes = std::fs::read(root.join("texture-iv-chroma-b-oracles.y16")).unwrap();
    let golden: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 132);
    let witnesses = manifest["rounding_witnesses"].as_array().unwrap();
    assert_eq!(witnesses.len(), 240);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let oracle = |role: usize, component: usize| -> &[u16] {
            let offset = case["offsets"][role][component].as_u64().unwrap() as usize;
            &golden[offset..offset + if component == 0 { 1024 } else { 256 }]
        };
        if !case["intra"].as_bool().unwrap() && case["mode"].as_str() == Some("bi") {
            for witness in witnesses {
                let c = witness["component"].as_u64().unwrap() as usize;
                let x = witness["x"].as_u64().unwrap() as usize;
                let y = witness["y"].as_u64().unwrap() as usize;
                let correct = witness["correct"].as_u64().unwrap() as u16;
                assert_eq!(oracle(0, c)[y * 16 + x], correct);
                assert_ne!(
                    correct,
                    witness["early"].as_u64().unwrap() as u16,
                    "{name}: rounding witness"
                );
            }
        }
        if case["explicit_chroma"].as_bool().unwrap() {
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
                    let header = &plan
                        .pictures()
                        .iter()
                        .find(|p| p.binding.sps.layer_id == 5)
                        .unwrap()
                        .slices[0];
                    assert_eq!(
                        u64::from(header.max_merge_candidates),
                        case["maximum"].as_u64().unwrap()
                    );
                    let weights = header.weights.as_ref().unwrap();
                    assert_eq!(weights.denominators, [1, 1]);
                    for list in 0..2 {
                        for reference in 0..3 {
                            for component in 1..3 {
                                let pair =
                                    &manifest["chroma_weights"][list][reference][component - 1];
                                assert_eq!(
                                    weights.lists[list][reference].values[component],
                                    pair[0].as_i64().unwrap() as i16,
                                    "{name}: parsed chroma weight"
                                );
                                assert_eq!(
                                    weights.lists[list][reference].offsets[component],
                                    pair[1].as_i64().unwrap() as i16,
                                    "{name}: midpoint-corrected offset"
                                );
                            }
                        }
                    }
                }
                let (next, transition, _, _) = plan.into_parts();
                params = next;
                if let Some(t) = transition {
                    state = t.into_state();
                }
            }
        }
        // Both chroma planes vary and differ from one another. IV's own-view
        // references also differ from the source-view pixels, so copying that
        // source picture cannot accidentally satisfy the color oracle.
        assert_ne!(oracle(0, 1), oracle(0, 2));
        assert_ne!(oracle(1, 1), oracle(1, 2));
        assert!(oracle(1, 1).windows(2).any(|v| v[0] != v[1]));
        if case["label"].as_str() == Some("IV") {
            assert_ne!(
                oracle(0, 1),
                oracle(1, 1),
                "{name}: copied source would be indistinguishable"
            );
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
                for component in 0..3 {
                    let plane = &frame.decoded.picture.planes[component];
                    assert_eq!(
                        plane.dimensions(),
                        if component == 0 { [32, 32] } else { [16, 16] }
                    );
                    if poc == 1 && (id == 3 || id == 5) {
                        assert_eq!(
                            plane.samples(),
                            oracle(usize::from(id == 5), component),
                            "{name}: layer{id} component{component}"
                        );
                    } else {
                        assert_eq!(
                            plane.samples(),
                            raw_plane(id, poc, component),
                            "{name}: raw layer{id} POC{poc} component{component}"
                        );
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
        let expected = |poc: i32| -> Vec<u8> {
            let mut packed = Vec::with_capacity(1536);
            for component in 0..3 {
                if poc == 1 {
                    packed.extend(oracle(1, component).iter().map(|&v| v as u8));
                } else {
                    packed.extend(raw_plane(5, poc, component).into_iter().map(|v| v as u8));
                }
            }
            packed
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
                let poc = frame.presentation_time.ticks as i32;
                seen.push(poc);
                assert_eq!(
                    frame.packed.unwrap().frame.data,
                    expected(poc),
                    "{name}: packed Y/Cb/Cr POC{poc}"
                );
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
                packed.frame.data,
                expected(poc),
                "{name}: seek all planes POC{poc}"
            );
        }
    }
}
