use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraTick, LatestFrame, NativeCameraSource, fit_bgra},
};
use std::io::Cursor;

#[test]
fn hevc_resolution_change_keeps_camera_format_pixels_and_timeline() {
    let file = include_bytes!("fixtures/playback-errors/hevc-camera-resize.mp4");
    let mut reference = NativeReader::software(Cursor::new(file), 16 << 20).unwrap();
    let mut expected = Vec::new();
    let mut sizes = Vec::new();
    while reference.read_frame().unwrap() {
        let dimensions = reference.dimensions();
        sizes.push(dimensions);
        let (start, _, scale) = reference.frame_interval().unwrap();
        let position = (start * 1_000_000_000).div_ceil(u128::from(scale)) as u64;
        assert_eq!(position, (expected.len() as u64 * 1_000_000_000).div_ceil(30));
        let bgra: Vec<u8> = reference
            .rgb()
            .chunks_exact(3)
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        let mut fitted = vec![0; 128 * 128 * 4];
        fit_bgra(&bgra, dimensions, &mut fitted, [128, 128]).unwrap();
        expected.push((position, fitted));
    }
    assert_eq!(
        sizes,
        [
            [128, 128],
            [128, 128],
            [128, 128],
            [96, 64],
            [96, 64],
            [96, 64]
        ]
    );
    let reader = NativeReader::software(Cursor::new(file), 16 << 20).unwrap();
    let mut source = NativeCameraSource::new(reader);
    let destination = LatestFrame::new(128, 128, 128 * 128 * 4).unwrap();
    let mut actual = vec![0; 128 * 128 * 4];
    for (sequence, (position, pixels)) in expected.iter().chain(expected.iter().take(1)).enumerate()
    {
        let tick = CameraTick {
            sequence: sequence as u64,
            host_time_ns: sequence as u64 + 1,
            media_time_ns: *position,
        };
        assert!(source.publish(tick, &destination).unwrap());
        assert_eq!(
            destination.copy_latest(None, &mut actual).unwrap(),
            Some(tick)
        );
        assert_eq!(&actual, pixels, "frame {sequence}");
        assert_eq!(destination.dimensions(), [128, 128]);
    }
}

#[test]
fn fitted_crop_aspect_and_invalid_publications_preserve_latest_frame() {
    let destination = LatestFrame::new(4, 4, 64).unwrap();
    let rgb = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let tick = CameraTick {
        sequence: 0,
        host_time_ns: 1,
        media_time_ns: 0,
    };
    destination
        .publish_rgb_fitted_aspect(tick, &rgb, [4, 1], [1, 0, 1, 0], [1, 1], [1, 1])
        .unwrap();
    let mut pixels = [0; 64];
    destination.copy_latest(None, &mut pixels).unwrap();
    assert_eq!(&pixels[16..24], &[0, 255, 0, 255, 0, 255, 0, 255]);
    assert_eq!(&pixels[24..32], &[255, 0, 0, 255, 255, 0, 0, 255]);
    let next = CameraTick {
        sequence: 1,
        host_time_ns: 2,
        ..tick
    };
    assert!(
        destination
            .publish_rgb_fitted_aspect(next, &rgb, [4, 1], [0; 4], [0, 1], [1, 1])
            .is_err()
    );
    assert!(
        destination
            .publish_rgb_fitted_aspect(next, &rgb[..11], [4, 1], [0; 4], [1, 1], [1, 1])
            .is_err()
    );
    let mut unchanged = [0; 64];
    assert_eq!(
        destination.copy_latest(None, &mut unchanged).unwrap(),
        Some(tick)
    );
    assert_eq!(unchanged, pixels);
    // Anamorphic destination samples preserve the original display aspect.
    destination
        .publish_rgb_fitted_aspect(next, &rgb, [4, 1], [1, 0, 1, 0], [1, 1], [2, 1])
        .unwrap();
    destination.copy_latest(None, &mut pixels).unwrap();
    assert_eq!(&pixels[..8], &[0, 255, 0, 255, 0, 255, 0, 255]);
}
