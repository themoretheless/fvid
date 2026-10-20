//! Real worker-sized DIM pictures and equivalent authored constrained-intra controls.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn queued_dim_cra_pictures_match_scalar_oracle_and_direct_controls() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let texture =
        std::fs::read(root.parent().unwrap().join("hevc-pcm-parallel-rext8.yuv")).unwrap();
    for family in ["queued-dim", "queued-dim-full", "queued-dim-dc"] {
        for residual in if family.ends_with("-dc") {
            &["zero"][..]
        } else {
            &["zero", "mixed"][..]
        } {
            for dlt in [false, true] {
                let stem = format!("{family}-{residual}{}", if dlt { "-dlt" } else { "" });
                let expected: Vec<u16> = std::fs::read(root.join(format!("{stem}.y16")))
                    .unwrap()
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                for direct in [false, true] {
                    let movie = format!("{stem}{}.mp4", if direct { "-direct" } else { "" });
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
                            let picture = &frame.decoded.picture;
                            assert_eq!(picture.dimensions, [128, 96]);
                            let actual = picture.planes[0].samples();
                            if frame.decoded.layer_id == 3 {
                                assert_eq!(actual.len(), expected.len());
                                assert_eq!(
                                    actual
                                        .iter()
                                        .zip(&expected)
                                        .enumerate()
                                        .find(|(_, (a, b))| a != b),
                                    None,
                                    "{movie}: first mismatching depth pixel"
                                );
                            } else {
                                assert!(
                                    actual
                                        .iter()
                                        .zip(&texture[..128 * 96])
                                        .all(|(&a, &b)| a == u16::from(b)),
                                    "rewritten synthetic CRA texture differs"
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
