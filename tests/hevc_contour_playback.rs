//! Actual coded contour selection, not merely enabled SPS metadata.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn contour_cra_uses_current_texture_pixels_with_and_without_dlt() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for (movie, oracle) in [
        ("contour-playback.mp4", "contour-picture.y16"),
        ("contour-dlt-playback.mp4", "contour-dlt.y16"),
    ] {
        let expected: Vec<u16> = std::fs::read(root.join(oracle))
            .unwrap()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(movie)).unwrap()),
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
