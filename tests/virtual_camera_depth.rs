use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraEndBehavior, CameraTick, LatestFrame, NativeCameraSource},
};
use std::io::Cursor;
const CASES: [(&[u8], &[u8]); 2] = [
    (
        include_bytes!("fixtures/playback-errors/camera-y4m-10.y4m"),
        include_bytes!("fixtures/playback-errors/camera-y4m-10-analytic.rgb"),
    ),
    (
        include_bytes!("fixtures/playback-errors/camera-y4m-16.y4m"),
        include_bytes!("fixtures/playback-errors/camera-y4m-16-analytic.rgb"),
    ),
];
#[test]
fn camera_high_depth_bgra_matches_rgb_reference_and_holds_or_rewinds_exactly() {
    for (data, analytic) in CASES {
        assert_eq!(analytic.len(), 2 * 8 * 8 * 3);
        assert_ne!(
            &analytic[..192],
            &analytic[192..],
            "fixture must distinguish both video frames"
        );
        let reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
        let mut source = NativeCameraSource::new(reader);
        let destination = LatestFrame::new(8, 8, 256).unwrap();
        let mut actual = vec![0; 256];
        let mut first = None;
        let mut second = None;
        for (sequence, (media, index)) in [
            (0, 0),
            (20_000_000, 0),
            (40_000_000, 1),
            (90_000_000, 1),
            (0, 0),
            (40_000_000, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let tick = CameraTick {
                sequence: sequence as u64,
                host_time_ns: sequence as u64 + 1,
                media_time_ns: media,
            };
            assert!(source.publish(tick, &destination).unwrap());
            assert_eq!(
                destination.copy_latest(None, &mut actual).unwrap(),
                Some(tick)
            );
            let reference = &analytic[index * 192..(index + 1) * 192];
            for (bgra, rgb) in actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<3>().0.iter()) {
                assert_eq!(bgra[3], 255);
                assert_eq!(
                    &[bgra[2], bgra[1], bgra[0]],
                    rgb,
                    "exact rational colour oracle at frame {index}"
                );
            }
            let saved = if index == 0 { &mut first } else { &mut second };
            if let Some(pixels) = saved {
                assert_eq!(&actual, pixels, "held/rewound frames must match exactly");
            } else {
                *saved = Some(actual.clone());
            }
        }
    }
}
#[test]
fn camera_high_depth_loop_returns_to_the_same_bgra_frames() {
    for (data, _) in CASES {
        let reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
        let mut source = NativeCameraSource::new(reader).with_end_behavior(CameraEndBehavior::Loop);
        let destination = LatestFrame::new(8, 8, 256).unwrap();
        let mut actual = vec![0; 256];
        let mut frames = Vec::new();
        for (sequence, media) in [0, 40_000_000, 90_000_000, 130_000_000]
            .into_iter()
            .enumerate()
        {
            source
                .publish(
                    CameraTick {
                        sequence: sequence as u64,
                        host_time_ns: sequence as u64 + 1,
                        media_time_ns: media,
                    },
                    &destination,
                )
                .unwrap();
            destination.copy_latest(None, &mut actual).unwrap();
            frames.push(actual.clone());
        }
        assert_ne!(frames[0], frames[1]);
        assert_eq!(frames[0], frames[2]);
        assert_eq!(frames[1], frames[3]);
    }
}
