use fvid_camera_ffi::{
    fvid_camera_close, fvid_camera_frame, fvid_camera_open, fvid_camera_set_loop, fvid_camera_size,
};
#[test]
fn c_api_preserves_high_depth_bgra_hold_seek_and_loop() {
    for (name, oracle) in [
        (
            "camera-y4m-10",
            include_bytes!("../../../tests/fixtures/playback-errors/camera-y4m-10-analytic.rgb")
                .as_slice(),
        ),
        (
            "camera-y4m-16",
            include_bytes!("../../../tests/fixtures/playback-errors/camera-y4m-16-analytic.rgb")
                .as_slice(),
        ),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/fixtures/playback-errors/{name}.y4m"));
        let path = path.to_str().unwrap().as_bytes();
        // Safety: live owned handle, serialized calls, valid disjoint buffers.
        unsafe {
            let handle = fvid_camera_open(path.as_ptr(), path.len(), 16 << 20);
            assert!(!handle.is_null());
            struct Close(*mut fvid_camera_ffi::CameraSource);
            impl Drop for Close {
                fn drop(&mut self) {
                    unsafe {
                        fvid_camera_close(self.0);
                    }
                }
            }
            let _close = Close(handle);
            let size = fvid_camera_size(handle);
            assert_eq!([size.width, size.height], [8, 8]);
            let mut pixels = vec![0; 256];
            for (sequence, (media, index)) in [(0, 0), (40_000_000, 1), (90_000_000, 1), (0, 0)]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    fvid_camera_frame(
                        handle,
                        media,
                        sequence as u64 + 1,
                        sequence as u64,
                        pixels.as_mut_ptr(),
                        pixels.len()
                    ),
                    1
                );
                let expected = &oracle[index * 192..(index + 1) * 192];
                for (bgra, rgb) in pixels.chunks_exact(4).zip(expected.chunks_exact(3)) {
                    assert_eq!(bgra, &[rgb[2], rgb[1], rgb[0], 255]);
                }
            }
            assert_eq!(fvid_camera_set_loop(handle, 1), 1);
            assert_eq!(
                fvid_camera_frame(handle, 90_000_000, 5, 4, pixels.as_mut_ptr(), pixels.len()),
                1
            );
            for (bgra, rgb) in pixels.chunks_exact(4).zip(oracle[..192].chunks_exact(3)) {
                assert_eq!(bgra, &[rgb[2], rgb[1], rgb[0], 255]);
            }
        }
    }
}
