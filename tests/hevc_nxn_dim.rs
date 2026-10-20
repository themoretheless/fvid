//! Real NxN CU mode interleaving, same-CU neighbours and residual reconstruction.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn mixed_nxn_dim_and_ordinary_prediction_blocks_play_exactly_and_rewind() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for prefix in ["nxn", "nxn4"] {
        for schedule in ["owco", "cowo", "wocw", "wcwc"] {
            for residual in ["zero", "mixed"] {
                let stem = format!("{prefix}-{schedule}-{residual}");
                for dlt in [false, true] {
                    let movie = format!("{stem}{}-playback.mp4", if dlt { "-dlt" } else { "" });
                    let expected: Vec<u16> = std::fs::read(root.join(format!(
                        "{stem}-{}.y16",
                        if dlt { "dlt" } else { "picture" }
                    )))
                    .unwrap()
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                    let mut reader = LayeredMp4VideoReader::open(
                        std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),
                        Limits::default(),
                        1,
                        0,
                        64 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut layers = Vec::new();
                        while let Some(frame) = reader.read_frame().unwrap() {
                            layers.push(frame.decoded.layer_id);
                            if frame.decoded.layer_id == 3 {
                                assert_eq!(
                                    frame.decoded.picture.coding_unit_at([0, 0]).unwrap(),
                                    (
                                        if prefix == "nxn4" { 3 } else { 5 },
                                        fvid::codec::hevc_inter_syntax::Partition::Quarters
                                    )
                                );
                                let actual = frame.decoded.picture.planes[0].samples();
                                assert_eq!(actual.len(), expected.len());
                                assert_eq!(
                                    actual
                                        .iter()
                                        .zip(&expected)
                                        .enumerate()
                                        .find(|(_, (a, b))| a != b),
                                    None,
                                    "{movie}: first mismatching pixel"
                                );
                            }
                        }
                        assert_eq!(layers, [0, 3]);
                        reader.rewind();
                    }
                }
            }
        }
    }
}
#[test]
fn nxn_dim_split_is_a_semantic_refusal_not_acceptance() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for schedule in ["owco", "wcwc"] {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(
                std::fs::read(root.join(format!("nxn-{schedule}-split-playback.mp4"))).unwrap(),
            ),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                reader.read_frame().err().unwrap().to_string(),
                "HEVC NxN DIM prediction block cannot split its transform"
            );
            reader.rewind();
        }
    }
}
