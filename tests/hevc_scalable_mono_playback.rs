//! Full coded AU/DPB/reference preparation, distinct from grid-only resampling.
use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredParameters},
        hevc_poc::LayeredPocState,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::LayeredMp4VideoReader,
};
#[test]
fn coded_scalable_mono_resampling_drives_temporal_mvp_and_sample_prediction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for integer_scale in [false, true] {
        for displacement in [4, 32767, -32768] {
            let stem = format!(
                "scalable-mono-resampling{}{}",
                if integer_scale { "-integer-scale" } else { "" },
                if displacement == 32767 {
                    "-positive-limit"
                } else if displacement == -32768 {
                    "-negative-limit"
                } else {
                    ""
                }
            );
            let data = std::fs::read(root.join(format!("{stem}.mp4"))).unwrap();
            let expected: Vec<_> = std::fs::read(root.join(format!("{stem}.y16")))
                .unwrap()
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            let mut input =
                Mp4Reader::open(std::io::Cursor::new(data.clone()), Limits::default()).unwrap();
            let params =
                LayeredParameters::from_configuration(&input.tracks()[0].configuration, 64 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            let plan = LayeredAccessUnit::parse(
                &params,
                &LayeredPocState::default(),
                &packet,
                4,
                1,
                64 << 20,
            )
            .unwrap();
            for (index, picture) in plan.pictures().iter().enumerate() {
                let binding = &picture.binding;
                assert!(!binding.sps.sps.depth_layer);
                assert!(binding.sps.sps.three_d.is_none());
                let profile = binding.sps.effective_profile.profile.unwrap();
                assert_eq!(profile.idc, if index == 0 { 1 } else { 10 });
                if index > 0 {
                    assert_eq!(
                        profile.constraints,
                        [43, 42, 41, 35, 34]
                            .iter()
                            .map(|bit| 1u64 << bit)
                            .sum::<u64>()
                    );
                }
                let graph = &binding.context().vps.extension.as_ref().unwrap().graph;
                assert_eq!(graph.scalability_ids[index][0], 0);
                assert_eq!(graph.scalability_ids[index][2], index as u8);
            }
            let mut reader = LayeredMp4VideoReader::open(
                std::io::Cursor::new(data),
                Limits::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap_or_else(|e| panic!("scalable-mono-resampling open: {e}"));
            for _ in 0..2 {
                let mut layers = Vec::new();
                let mut targets = 0;
                while let Some(frame) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{stem} after layers{layers:?}: {e}"))
                {
                    layers.push(frame.decoded.layer_id);
                    if frame.decoded.layer_id == 0 {
                        assert!(
                            frame.decoded.picture.planes[0]
                                .samples()
                                .iter()
                                .all(|&v| v == 128)
                        );
                    }
                    if frame.decoded.layer_id != 5 {
                        continue;
                    }
                    assert_eq!(frame.decoded.poc, targets);
                    let picture = &frame.decoded.picture;
                    assert_eq!(picture.dimensions, [64, 64]);
                    let wanted = &expected[targets as usize * 4096..(targets as usize + 1) * 4096];
                    assert_eq!(
                        picture.planes[0]
                            .samples()
                            .iter()
                            .zip(wanted)
                            .enumerate()
                            .find(|(_, (a, b))| a != b),
                        None,
                        "{stem}: target POC{targets}"
                    );
                    if targets > 0 {
                        for y in (0..64).step_by(8) {
                            for x in (0..64).step_by(8) {
                                let motion = picture.prediction_motion_at([x, y]).unwrap();
                                let projected = if displacement == 4 {
                                    if integer_scale { 8 } else { 6 }
                                } else {
                                    displacement
                                };
                                assert_eq!(
                                    motion[0].unwrap().mv,
                                    if targets == 1 { [0, projected] } else { [0, 0] },
                                    "{stem}: target POC{targets} motion{x},{y}"
                                );
                                assert_eq!(
                                    motion[0].unwrap().reference,
                                    if targets == 1 { 0 } else { 1 },
                                    "{stem}: chosen reference"
                                );
                                assert!(motion[1].is_none());
                            }
                        }
                    }
                    targets += 1;
                }
                assert_eq!(layers, [0, 3, 5, 0, 3, 5, 0, 3, 5]);
                assert_eq!(targets, 3);
                reader.rewind();
            }
        }
    }
}
