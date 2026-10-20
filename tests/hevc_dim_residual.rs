//! Coded DIM transform/CBF/coefficients, with independently authored DC-DCT oracles.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn coded_wedge_and_contour_residuals_add_after_prediction_and_dlt() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for mode in ["contour", "wedge"] {
        for residual in [
            "zero",
            "positive",
            "negative",
            "positive-clip",
            "negative-clip",
        ] {
            for dlt in [false, true] {
                let stem = format!("{mode}-residual-{residual}");
                let movie = format!("{stem}{}-playback.mp4", if dlt { "-dlt" } else { "" });
                let oracle = format!("{stem}-{}.y16", if dlt { "dlt" } else { "picture" });
                let expected: Vec<u16> = std::fs::read(root.join(oracle))
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
                                frame.decoded.picture.planes[0].samples(),
                                expected,
                                "{movie}"
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

#[test]
fn dim_split_transform_is_a_semantic_refusal_not_playback_acceptance() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for mode in ["contour", "wedge"] {
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(
                std::fs::read(root.join(format!("{mode}-residual-split-playback.mp4"))).unwrap(),
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
                "HEVC residual DIM prediction block cannot split its transform"
            );
            reader.rewind();
        }
    }
}
