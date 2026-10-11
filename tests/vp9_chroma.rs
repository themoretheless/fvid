use fvid::codec::{
    vp9::HeaderState,
    vp9_decoder::Decoder,
    vp9_probs::{CompressedHeader, Probabilities},
};
fn packet(chroma: &str, depth: u8) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}-synthetic.vp9",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn owned_neutral_420_controls_decode_every_sample() {
    for depth in [8, 10, 12] {
        let p = packet("420", depth);
        let frame = Decoder::new(32 << 20).decode(&p).unwrap();
        assert_eq!(frame.picture.size, [64, 64]);
        assert_eq!(frame.picture.depth, depth);
        for (i, plane) in frame.picture.planes.iter().enumerate() {
            assert_eq!(
                [plane.width, plane.height],
                if i == 0 { [64, 64] } else { [32, 32] }
            );
            assert!(plane.samples.iter().all(|&v| v == 1 << (depth - 1)));
        }
    }
}
#[test]
fn full_chroma_headers_reach_the_specific_reconstruction_gap() {
    for depth in [8, 10, 12] {
        for chroma in ["422", "444"] {
            let p = packet(chroma, depth);
            let h = HeaderState::default().parse(&p).unwrap();
            assert_eq!(
                h.picture.format.subsampling,
                if chroma == "422" {
                    [true, false]
                } else {
                    [false, false]
                }
            );
            assert!(h.lossless());
            CompressedHeader::parse(&p, &h, &Probabilities::default()).unwrap();
            let mut d = Decoder::new(32 << 20);
            let e = d.decode(&p).err().unwrap();
            assert_eq!(
                e.to_string(),
                "VP9 picture decoder requires coded 4:2:0 frames, single-reference prediction and no segmentation"
            );
            assert_eq!(
                d.decode(&p).err().unwrap().to_string(),
                "VP9 decoder requires reset after a decode error"
            );
        }
    }
}
