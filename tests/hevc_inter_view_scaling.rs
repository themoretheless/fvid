use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn coded_three_view_control_preserves_temporal_reference_identity() {
    check_cases(false);
}
fn check_cases(enabled: bool) {
    for (label, source, target) in [
        ("", 4, 16),
        ("negative", 4, -8),
        ("equal", 4, 4),
        ("zero-source", 4, 4),
        ("zero-target", 4, 4),
        ("positive-limit", 32767, 32767),
        ("negative-limit", -32768, -32768),
        ("factor-clip", 4, 64),
    ] {
        for merge in [false, true] {
            check(enabled, label, source, target, merge);
        }
    }
}
fn check(enabled: bool, label: &str, source: i32, target: i32, merge: bool) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/playback-errors/hevc-wedge/inter-view-temporal-scaling-{}{}{}.mp4",
        if enabled { "on" } else { "off" },
        if label.is_empty() {
            String::new()
        } else {
            format!("-{label}")
        },
        if merge { "-merge" } else { "" }
    ));
    let data = std::fs::read(path).unwrap();
    let mut input = fvid::container::mp4::Mp4Reader::open(
        std::io::Cursor::new(data.clone()),
        Limits::default(),
    )
    .unwrap();
    let params = fvid::codec::hevc_decoder::LayeredParameters::from_configuration(
        &input.tracks()[0].configuration,
        64 << 20,
    )
    .unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let plan = fvid::codec::hevc_decoder::LayeredAccessUnit::parse(
        &params,
        &fvid::codec::hevc_poc::LayeredPocState::default(),
        &packet,
        4,
        1,
        64 << 20,
    )
    .unwrap();
    assert_eq!(plan.pictures().len(), 3);
    let views = match label {
        "negative" => [100, 102, 96],
        "equal" => [7, 7, 7],
        "zero-source" => [100, 100, 108],
        "zero-target" => [100, 102, 100],
        "factor-clip" => [0, 1, 32767],
        _ => [100, 102, 108],
    };
    for (index, picture) in plan.pictures().iter().enumerate() {
        let binding = &picture.binding;
        assert_eq!(
            binding.sps.effective_profile.profile.unwrap().idc,
            if index == 0 { 1 } else { 8 }
        );
        assert!(!binding.sps.sps.depth_layer);
        assert_eq!(binding.sps.sps.chroma_format, 1);
        assert_eq!(binding.sps.sps.depth, [8, 8]);
        assert_eq!(binding.sps.sps.dimensions, [32, 32]);
        assert!(binding.pps.reference_locations.is_empty());
        let ctx = binding.context();
        let graph = &ctx.vps.extension.as_ref().unwrap().graph;
        assert!(ctx.vps.future_extension); // explicitly authored VPS extension2/3D payload
        assert_eq!(graph.layer_ids, [0, 3, 5]);
        assert_eq!(graph.view_ids, views);
        assert_eq!(graph.scalability_ids[index][1], index as u8);
        assert_eq!(graph.scalability_ids[index][0], 0);
        assert!(graph.scalability_ids[index][2..].iter().all(|&d| d == 0));
        if index == 2 {
            assert_eq!(
                binding
                    .sps
                    .sps
                    .three_d
                    .as_ref()
                    .unwrap()
                    .inter_view_mv_scaling[0],
                enabled
            );
        }
    }
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(data.clone()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut seen = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            let id = frame.decoded.layer_id;
            let poc = frame.decoded.poc;
            seen.push((id, poc));
            let vector = if poc == 0 || id == 0 {
                0
            } else if id == 5 && enabled && merge {
                target
            } else {
                source
            };
            let shift = vector.div_euclid(4);
            for y in 0..32usize {
                for x in 0..32usize {
                    if vector.rem_euclid(4) != 0 {
                        // The signed-limit cases put every interpolation tap beyond
                        // the picture edge, so their scalar oracle is exact too.
                        assert!(y as i32 + shift - 3 >= 31 || y as i32 + shift + 4 <= 0);
                    }
                    let row = (y as i32 + shift).clamp(0, 31) as usize;
                    let value = ((x * 3 + row * 5 + 17) % 256) as u16;
                    assert_eq!(
                        frame.decoded.picture.planes[0].samples()[y * 32 + x],
                        value,
                        "layer{id} poc{poc} at{x},{y}"
                    );
                }
            }
            if poc == 1 && (id == 3 || id == 5) {
                let mv = frame.decoded.picture.prediction_motion_at([0, 0]).unwrap();
                assert_eq!(mv[0].unwrap().mv, [0, i64::from(vector)]);
                assert!(mv[1].is_none());
                use fvid::codec::hevc_motion::texture::Identity;
                let target = [Identity { poc: 1, view: 0 }];
                let candidate = frame
                    .decoded
                    .picture
                    .texture_merge_candidate_at([0, 0], [&target, &[]], false, |id| {
                        Ok(match id {
                            0 => 0,
                            3 => 1,
                            5 => 2,
                            _ => panic!("unknown reference layer"),
                        })
                    })
                    .unwrap();
                assert_eq!(candidate[0].unwrap().reference, 0);
                assert_eq!(
                    candidate[0].unwrap().samples,
                    [0, ((vector + 2) >> 2) as i16]
                );
                assert!(candidate[1].is_none());
            }
        }
        assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
        reader.rewind();
    }
    // Exercise automatic output-layer selection and the ordinary player entry.
    let predicted = if enabled && merge { target } else { source };
    let y_plane = |index: usize| -> Vec<u8> {
        (0..32i32)
            .flat_map(|y| {
                (0..32i32).map(move |x| {
                    let row = if index == 0 {
                        y
                    } else {
                        (y + predicted.div_euclid(4)).clamp(0, 31)
                    };
                    ((3 * x + 5 * row + 17) % 256) as u8
                })
            })
            .collect()
    };
    let mut common = fvid::playback::video::mp4::Mp4VideoReader::open_software(
        std::io::Cursor::new(data.clone()),
        Limits::default(),
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut count = 0;
        while let Some(frame) = common.read_frame().unwrap() {
            assert_eq!(common.selected_layer_id(), Some(5));
            let packed = frame.packed.unwrap();
            assert_eq!(&packed.frame.data[..1024], y_plane(frame.sample_index));
            assert!(packed.frame.data[1024..].iter().all(|&v| v == 128));
            count += 1;
        }
        assert_eq!(count, 2);
        common.rewind();
    }
    use fvid::playback::video::native::{NativeReader, RawFrame};
    let mut common = NativeReader::software(std::io::Cursor::new(data), 64 << 20).unwrap();
    let frame = common
        .seek_raw(std::time::Duration::from_millis(40))
        .unwrap()
        .unwrap();
    let RawFrame::Planar(packed) = frame else {
        panic!("expected native planar frame")
    };
    assert_eq!(&packed.frame.data[..1024], y_plane(1));
}
#[test]
fn coded_three_view_scaling_reconstructs_numeric_view_distance() {
    check_cases(true);
}
