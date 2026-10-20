//! Ordinary axis/DC predictors and transform splits beside wedge/contour PBs.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn ordinary_angular_and_split_transforms_mix_with_dim_exactly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for (mode, split) in [(1, true), (10, false), (10, true), (26, false), (26, true)] {
        for schedule in ["owco", "cowo", "wocw"] {
            for residual in ["zero", "mixed"] {
                let stem = format!(
                    "nxn-mode{mode}-{}-{schedule}-{residual}",
                    if split { "split" } else { "whole" }
                );
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
