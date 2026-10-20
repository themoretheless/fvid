use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn coded_depth_view_control_preserves_integer_motion() {
    check_cases(false);
}
fn check_cases(enabled: bool) {
    for (label, source, target) in [
        ("", 1, 4),
        ("negative", 1, -2),
        ("equal", 1, 1),
        ("zero-source", 1, 1),
        ("zero-target", 1, 1),
        ("positive-limit", 32767, 32767),
        ("negative-limit", -32768, -32768),
        ("factor-clip", 1, 16),
        ("integer-round", 1, 1),
        ("negative-round", -1, -1),
    ] {
        for merge in [false, true] {
            for (size, direct) in [(32usize, false), (128, false), (128, true)] {
                check(enabled, label, source, target, merge, size, direct);
            }
        }
    }
}
fn check(
    enabled: bool,
    label: &str,
    source: i32,
    target: i32,
    merge: bool,
    size: usize,
    direct: bool,
) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/playback-errors/hevc-wedge/depth-view-temporal-scaling{}-{}{}{}{}.mp4",
        if size == 32 {
            String::new()
        } else {
            format!("-{size}")
        },
        if enabled { "on" } else { "off" },
        if label.is_empty() {
            String::new()
        } else {
            format!("-{label}")
        },
        if merge { "-merge" } else { "" },
        if direct { "-direct" } else { "" }
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
    assert_eq!(plan.pictures().len(), 4);
    let views = match label {
        "negative" => [100, 102, 96],
        "equal" => [7, 7, 7],
        "zero-source" => [100, 100, 108],
        "zero-target" => [100, 102, 100],
        "factor-clip" => [0, 1, 32767],
        "integer-round" | "negative-round" => [100, 103, 102],
        _ => [100, 102, 108],
    };
    for (index, picture) in plan.pictures().iter().enumerate() {
        let binding = &picture.binding;
        assert_eq!(
            binding.sps.effective_profile.profile.unwrap().idc,
            if index == 0 { 1 } else { 8 }
        );
        assert_eq!(binding.sps.sps.depth_layer, index != 0);
        assert_eq!(
            binding.sps.sps.chroma_format,
            if index == 0 { 1 } else { 0 }
        );
        assert_eq!(binding.sps.sps.depth, [8, 8]);
        assert_eq!(binding.sps.sps.dimensions, [size as u32, size as u32]);
        assert!(binding.pps.reference_locations.is_empty());
        if index > 0 {
            assert_eq!(binding.pps.pps.constrained_intra, direct);
        }
        let ctx = binding.context();
        let graph = &ctx.vps.extension.as_ref().unwrap().graph;
        assert!(ctx.vps.future_extension); // explicitly authored VPS extension2/3D payload
        assert_eq!(graph.layer_ids, [0, 3, 5, 8]);
        assert_eq!(graph.reference_list_layers(3).unwrap(), 0);
        assert_eq!(graph.reference_list_layers(5).unwrap(), 1 << 3);
        assert_eq!(graph.reference_list_layers(8).unwrap(), (1 << 3) | (1 << 5));
        assert_eq!(
            graph.depth_layer_ids().unwrap().collect::<Vec<_>>(),
            [3, 5, 8]
        );
        assert_eq!(graph.view_ids, views);
        assert_eq!(
            graph.scalability_ids[index][1],
            index.saturating_sub(1) as u8
        );
        assert_eq!(graph.scalability_ids[index][0], u8::from(index != 0));
        assert!(graph.scalability_ids[index][2..].iter().all(|&d| d == 0));
        if index == 3 {
            assert_eq!(
                binding
                    .sps
                    .sps
                    .three_d
                    .as_ref()
                    .unwrap()
                    .inter_view_mv_scaling[1],
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
            let vector = if poc == 0 || id == 0 || id == 3 {
                0
            } else if id == 8 && enabled && merge {
                target
            } else {
                source
            };
            let shift = vector;
            for y in 0..size {
                for x in 0..size {
                    let row = (y as i32 + shift).clamp(0, size as i32 - 1) as usize;
                    let value = if id == 0 {
                        128
                    } else {
                        ((x * 7 + row * 11 + 37) % 256) as u16
                    };
                    assert_eq!(
                        frame.decoded.picture.planes[0].samples()[y * size + x],
                        value,
                        "layer{id} poc{poc} at{x},{y}"
                    );
                }
            }
            if poc == 1 && (id == 5 || id == 8) {
                for y in (0..size).step_by(8) {
                    for x in (0..size).step_by(8) {
                        let mv = frame
                            .decoded
                            .picture
                            .prediction_motion_at([x as u32, y as u32])
                            .unwrap();
                        assert_eq!(mv[0].unwrap().reference, 0);
                        assert_eq!(
                            mv[0].unwrap().mv,
                            [0, i64::from(vector) * 4],
                            "{label}: layer{id} motion{x},{y}"
                        );
                        assert!(mv[1].is_none());
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
                (8, 0),
                (0, 1),
                (3, 1),
                (5, 1),
                (8, 1)
            ]
        );
        reader.rewind();
    }
    // Exercise automatic output-layer selection and the ordinary player entry.
    let predicted = if enabled && merge { target } else { source };
    let y_plane = |index: usize| -> Vec<u8> {
        (0..size as i32)
            .flat_map(|y| {
                (0..size as i32).map(move |x| {
                    let row = if index == 0 {
                        y
                    } else {
                        (y + predicted).clamp(0, size as i32 - 1)
                    };
                    ((7 * x + 11 * row + 37) % 256) as u8
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
            assert_eq!(common.selected_layer_id(), Some(8));
            let packed = frame.packed.unwrap();
            assert_eq!(
                &packed.frame.data[..size * size],
                y_plane(frame.sample_index)
            );
            assert!(packed.frame.data[size * size..].iter().all(|&v| v == 128));
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
    assert_eq!(&packed.frame.data[..size * size], y_plane(1));
}
#[test]
fn coded_depth_view_scaling_preserves_rounding_and_full_integer_range() {
    check_cases(true);
}
