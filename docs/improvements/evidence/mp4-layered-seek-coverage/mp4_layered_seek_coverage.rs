#![cfg(feature = "player")]
use fvid::playback_native::NativeReader;
use std::{io::Cursor, time::Duration};
fn open(name: &str) -> NativeReader<Cursor<Vec<u8>>> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-layered-seek-coverage")
            .join(name),
    )
    .unwrap();
    NativeReader::software(Cursor::new(bytes), 64 << 20).unwrap()
}
#[test]
fn layered_sequential_control_preserves_four_presentation_intervals() {
    for name in ["ordered.mp4", "reordered.mp4"] {
        let mut r = open(name);
        assert_eq!(r.duration(), Some(Duration::from_millis(160)));
        let oracle = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv"),
        )
        .unwrap();
        for i in 0..4 {
            let raw = r.read_frame_raw().unwrap().unwrap();
            let fvid::playback_native::RawFrame::Planar(frame) = raw else {
                panic!("expected native packed HEVC planes");
            };
            // The seed oracle contains three pictures; this witness repeats
            // only its first access unit, a 64x64 8-bit 4:2:0 picture.
            assert_eq!(frame.frame.data.len(), 64 * 64 * 3 / 2);
            assert_eq!(
                frame.frame.data,
                oracle[..64 * 64 * 3 / 2],
                "{name} frame {i}"
            );
            assert_eq!(r.frame_interval(), Some((i, i + 1, 25)), "{name}");
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        assert_eq!(r.frame_interval(), Some((0, 1, 25)));
    }
}
#[test]
fn layered_seek_preserves_frames_preceding_the_landing_in_decode_order() {
    for name in ["ordered.mp4", "reordered.mp4"] {
        let mut sequential = open(name);
        let expected: Vec<_> = (0..4)
            .map(|_| {
                sequential
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(64 << 20)
                    .unwrap()
            })
            .collect();
        let mut r = open(name);
        for ms in [0, 39, 40, 79, 80, 119, 120, 159, 160, 200] {
            let got = r.seek_raw(Duration::from_millis(ms)).unwrap().unwrap();
            let i = ms.min(159) / 40;
            assert_eq!(got.into_rgb(64 << 20).unwrap(), expected[i as usize]);
            assert_eq!(
                r.frame_interval(),
                Some((u128::from(i), u128::from(i + 1), 25)),
                "{name} seek {ms}"
            );
            if ms >= 160 {
                assert!(r.read_frame_raw().unwrap().is_none());
                assert!(r.read_frame_raw().unwrap().is_none());
            }
        }
    }
}
