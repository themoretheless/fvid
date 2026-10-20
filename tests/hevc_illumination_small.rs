use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::LayeredMp4VideoReader,
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, time::Duration};
#[test]
fn eight_sample_ic_cus_keep_chroma_on_ordinary_prediction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let data = std::fs::read(root.join("illumination-small-cu.mp4")).unwrap();
    let golden: Vec<u16> = std::fs::read(root.join("illumination-small-cu.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let mut reader =
        LayeredMp4VideoReader::open(Cursor::new(data.clone()), Limits::default(), 1, 0, 64 << 20)
            .unwrap();
    for _ in 0..2 {
        let mut seen = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            let id = frame.decoded.layer_id;
            let poc = frame.decoded.poc;
            seen.push((id, poc));
            if id != 5 || poc != 1 {
                continue;
            }
            let picture = &frame.decoded.picture;
            let mut at = 0;
            for c in 0..3 {
                let samples = picture.planes[c].samples();
                for (i, (&a, &b)) in samples
                    .iter()
                    .zip(&golden[at..at + samples.len()])
                    .enumerate()
                {
                    assert_eq!(a, b, "8x8 IC: c{c} sample{i}");
                }
                at += samples.len();
            }
            for y in (0..32).step_by(8) {
                for x in (0..32).step_by(8) {
                    let motion = picture.prediction_motion_at([x, y]).unwrap();
                    assert_eq!(
                        motion[0].unwrap().reference,
                        if x == 0 && y == 0 { 0 } else { 1 }
                    );
                    assert_eq!(motion[0].unwrap().mv, [0, 0]);
                    assert!(motion[1].is_none());
                }
            }
        }
        assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
        reader.rewind();
    }
    let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
    for _ in 0..2 {
        let RawFrame::Planar(frame) = native.seek_raw(Duration::from_millis(40)).unwrap().unwrap()
        else {
            panic!("planar small IC")
        };
        assert_eq!(
            frame.frame.data,
            golden.iter().map(|&v| v as u8).collect::<Vec<_>>()
        );
    }
}
