use fvid::playback_native::{NativeReader, RawFrame};
use std::{io::Cursor, time::Duration};

fn planar(frame: RawFrame) -> Vec<u8> {
    match frame {
        RawFrame::Avc { picture, .. } => {
            assert_eq!(picture.bit_depth, 8);
            assert_eq!(picture.dimensions(), (96, 64));
            let mut bytes = Vec::new();
            picture.write_planar(&mut bytes).unwrap();
            bytes
        }
        RawFrame::Planar8(p) => {
            assert_eq!((p.width, p.height), (96, 64));
            [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()].concat()
        }
        _ => panic!("expected native 8-bit planar decode"),
    }
}

fn check(video: &[u8], oracle: &[u8]) {
    const FRAME_BYTES: usize = 96 * 64 * 3 / 2;
    assert_eq!(oracle.len(), 12 * FRAME_BYTES);
    let mut reader = NativeReader::software(Cursor::new(video), 16 << 20).unwrap();
    assert!(!reader.hardware_accelerated());
    for pass in 0..2 {
        let mut previous = None;
        for (index, expected) in oracle.as_chunks::<FRAME_BYTES>().0.iter().enumerate() {
            let frame = reader.read_frame_raw().unwrap().expect("missing frame");
            assert_eq!(planar(frame), expected, "pass {pass}, frame {index}");
            let interval = reader.frame_interval().unwrap();
            assert!(interval.1 > interval.0);
            if let Some(start) = previous {
                assert!(interval.0 > start, "presentation order");
            }
            previous = Some(interval.0);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind().unwrap();
    }
    for millis in [0, 250, 600, 900] {
        let frame = reader
            .seek_raw(Duration::from_millis(millis))
            .unwrap()
            .unwrap();
        let (start, end, scale) = reader.frame_interval().unwrap();
        let target = u128::from(millis) * u128::from(scale);
        assert!(start * 1000 <= target && target < end * 1000);
        let index = ((start * 12 + u128::from(scale) / 2) / u128::from(scale)) as usize;
        assert_eq!(
            planar(frame),
            &oracle[index * FRAME_BYTES..(index + 1) * FRAME_BYTES],
            "seek {millis} ms"
        );
    }
}

#[test]
fn avc_baseline_short_clip() {
    check(
        include_bytes!("fixtures/short/avc-baseline.mp4"),
        include_bytes!("fixtures/short/avc-baseline.yuv"),
    );
}

#[test]
fn avc_bframes_short_clip() {
    check(
        include_bytes!("fixtures/short/avc-bframes.mp4"),
        include_bytes!("fixtures/short/avc-bframes.yuv"),
    );
}

#[test]
fn vp9_motion_short_clip() {
    check(
        include_bytes!("fixtures/short/vp9-motion.webm"),
        include_bytes!("fixtures/short/vp9-motion.yuv"),
    );
}
