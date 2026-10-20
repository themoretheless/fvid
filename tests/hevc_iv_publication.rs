use fvid::{
    codec::hevc_motion::disparity::derivation, playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, path::Path};

#[test]
fn native_selected_iv_publishes_camera_disparity_and_neighbours_without_external_labels() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let mut total = 0;
    for (manifest_name, b_slice) in [
        ("depth-iv-manifest.json", false),
        ("depth-iv-b-manifest.json", true),
    ] {
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(manifest_name)).unwrap()).unwrap();
        for case in manifest["cases"].as_array().unwrap() {
            total += 1;
            let name = case["file"].as_str().unwrap();
            let size = case["size"].as_u64().unwrap() as u32;
            let selected = case["enabled"].as_bool().unwrap()
                && !case[if b_slice { "intra" } else { "intra_source" }]
                    .as_bool()
                    .unwrap()
                && case["index"].as_u64().unwrap() == 0;
            let horizontal = if name == "depth-iv-camera-left.mp4" {
                -48
            } else if name == "depth-iv-camera-right.mp4" {
                0
            } else {
                1
            };
            let mut reader = LayeredMp4VideoReader::open(
                Cursor::new(std::fs::read(root.join(name)).unwrap()),
                Default::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut pictures = 0;
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                {
                    pictures += 1;
                    let picture = &frame.decoded.picture;
                    assert_eq!(
                        picture.nal_header(),
                        frame.decoded.nal,
                        "{name}: retained NAL header"
                    );
                    let graph = &frame.decoded.binding.vps.extension.as_ref().unwrap().graph;
                    let reference = fvid::codec::hevc_motion::Reference {
                        layer_id: frame.decoded.layer_id,
                        poc: frame.decoded.poc,
                        long_term: false,
                        picture: Some(picture.clone()),
                    };
                    let metadata = reference.disparity_metadata(graph).unwrap();
                    assert_eq!(
                        metadata.view_index,
                        graph.view_index(frame.decoded.layer_id).unwrap()
                    );
                    assert_eq!(metadata.temporal_id, frame.decoded.nal.temporal_id);
                    assert_eq!(metadata.irap, frame.decoded.nal.is_irap());
                    assert_eq!(metadata.poc, frame.decoded.poc);
                    let mut wrong = reference.clone();
                    wrong.layer_id = if reference.layer_id == 0 { 3 } else { 0 };
                    assert!(wrong.disparity_metadata(graph).is_err());
                    wrong = reference;
                    wrong.picture = None;
                    assert!(wrong.disparity_metadata(graph).is_err());
                    let expected =
                        selected && frame.decoded.layer_id == 8 && frame.decoded.poc == 1;
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let state = picture.inter_view_disparity_at([x, y]).unwrap();
                            assert_eq!(state.is_some(), expected, "{name}: saved IV at{x},{y}");
                            let neighbour = derivation::published_neighbour_from_picture(
                                picture,
                                [x, y],
                                |layer| graph.view_index(layer),
                            )
                            .unwrap();
                            if let Some(state) = state {
                                assert_eq!(state.vector.reference_view, 0);
                                assert_eq!(
                                    state.vector.mv,
                                    [horizontal, 0],
                                    "{name}: base disparity"
                                );
                                assert_eq!(
                                    state.refined_mv, state.vector.mv,
                                    "{name}: depth default has no separate refinement"
                                );
                                let neighbour = neighbour.unwrap();
                                assert!(neighbour.skip && neighbour.inter_view);
                                assert_eq!(neighbour.refined, Some(state.vector));
                                // Current POC deliberately differs from both saved
                                // temporal reference POCs: force saved-IV fallback.
                                let chosen = derivation::texture(
                                    [size, 0],
                                    8,
                                    100,
                                    true,
                                    2,
                                    5,
                                    None,
                                    None,
                                    |point| {
                                        Ok(if point[0] == i64::from(size) - 1 {
                                            Some(neighbour)
                                        } else {
                                            None
                                        })
                                    },
                                )
                                .unwrap();
                                assert_eq!(chosen, Some(state.vector));
                            } else if let Some(neighbour) = neighbour {
                                assert!(!neighbour.inter_view);
                                assert!(neighbour.refined.is_none());
                            }
                        }
                    }
                    assert!(picture.inter_view_disparity_at([size, 0]).is_err());
                }
                assert_eq!(pictures, if b_slice { 12 } else { 8 });
                reader.rewind();
            }
        }
    }
    assert_eq!(total, 90);
}
