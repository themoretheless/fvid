use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredParameters},
        hevc_poc::LayeredPocState,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::LayeredMp4VideoReader,
};
#[test]
fn six_texture_merge_candidates_select_texture_and_sixth_zero_candidate() {
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for index in [0, 5] {
            for intra_source in [false, true] {
                let name = format!(
                    "texture-motion{}-on{}{}-merge6-{index}.mp4",
                    if size == 32 {
                        String::new()
                    } else {
                        format!("-{size}")
                    },
                    if intra_source { "-intra-source" } else { "" },
                    if direct { "-direct" } else { "" }
                );
                let data = std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/playback-errors/hevc-wedge")
                        .join(&name),
                )
                .unwrap();
                let mut input =
                    Mp4Reader::open(std::io::Cursor::new(data.clone()), Limits::default()).unwrap();
                let mut params = LayeredParameters::from_configuration(
                    &input.tracks()[0].configuration,
                    64 << 20,
                )
                .unwrap();
                let mut state = LayeredPocState::default();
                let mut packet = Vec::new();
                for sample in 0..3 {
                    input.read_packet(0, sample, &mut packet).unwrap();
                    let plan =
                        LayeredAccessUnit::parse(&params, &state, &packet, 4, 1, 64 << 20).unwrap();
                    for p in plan.pictures() {
                        if p.binding.sps.layer_id == 3 && p.poc > 0 {
                            assert_eq!(p.slices[0].max_merge_candidates, 6, "{name}: I-35/I-36");
                        }
                    }
                    let (updated, transition, _, _) = plan.into_parts();
                    params = updated;
                    if let Some(t) = transition {
                        state = t.into_state();
                    }
                }
                let mut r = LayeredMp4VideoReader::open(
                    std::io::Cursor::new(data),
                    Limits::default(),
                    1,
                    0,
                    64 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut previous: Vec<u16> = (0..size)
                        .flat_map(|y| (0..size).map(move |x| ((7 * x + 11 * y + 37) % 256) as u16))
                        .collect();
                    let mut seen = Vec::new();
                    while let Some(f) = r.read_frame().unwrap() {
                        let layer = f.decoded.layer_id;
                        let poc = f.decoded.poc;
                        seen.push((layer, poc));
                        let mut expected = Vec::new();
                        for y in 0..size {
                            for x in 0..size {
                                let motion = if index == 0 && poc > 0 && !intra_source {
                                    if y % 32 < 8 { 1 } else { -1 }
                                } else {
                                    0
                                };
                                let row = (y as i32 + motion).clamp(0, size as i32 - 1) as usize;
                                let value = if layer == 0 {
                                    128
                                } else {
                                    previous[row * size + x]
                                };
                                assert_eq!(
                                    f.decoded.picture.planes[0].samples()[y * size + x],
                                    value,
                                    "{name}: layer{layer} poc{poc} at{x},{y}"
                                );
                                expected.push(value);
                                if layer == 3 && poc > 0 && x % 8 == 0 && y % 8 == 0 {
                                    let mv = f
                                        .decoded
                                        .picture
                                        .prediction_motion_at([x as u32, y as u32])
                                        .unwrap();
                                    assert_eq!(mv[0].unwrap().mv, [0, i64::from(motion) * 4]);
                                    if let Some(v) = mv[1] {
                                        assert_eq!(v.mv, [0, 0]);
                                        assert!(index == 5 || intra_source);
                                    }
                                }
                            }
                        }
                        if layer == 3 {
                            previous = expected;
                        }
                    }
                    assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1), (0, 2), (3, 2)]);
                    r.rewind();
                }
            }
        }
    }
}
