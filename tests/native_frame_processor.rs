use fvid::{
    native_geometry::{GeometryFrame, VideoGeometry},
    native_lossless, native_pixels,
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs::File,
    io::{BufReader, Cursor},
    path::Path,
};
#[test]
fn owned_processor_composites_before_ffv1_and_preserves_timing_depth() {
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let mut expected = Vec::new();
        let mut times = Vec::new();
        let mut input =
            NativeReader::software(BufReader::new(File::open(&source).unwrap()), usize::MAX)
                .unwrap();
        while let Some(frame) = input.read_frame_raw().unwrap() {
            let depth = match &frame {
                RawFrame::Avc { picture, .. } => picture.bit_depth,
                RawFrame::Planar8(_) => 8,
                _ => panic!("unexpected frame"),
            };
            let [w, h] = input.dimensions();
            let mut samples = VideoGeometry::default()
                .apply_display_media(&frame, w, h, 0)
                .unwrap();
            let bytes = if depth == 8 { 1 } else { 2 };
            let foreground = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: samples.subsampling,
                data: vec![0; 3 * bytes],
            };
            native_pixels::overlay_opaque(&mut samples, &foreground, depth, 0, 0).unwrap();
            expected.push((samples.data, depth));
            let (start, _, scale) = input.frame_interval().unwrap();
            times.push((start * 1_000_000_000 / u128::from(scale)) as u64);
        }
        let mut clock = Vec::new();
        let mut processor = |samples: &mut GeometryFrame, depth: u8, pts: u64| {
            clock.push(pts);
            let bytes = if depth == 8 { 1 } else { 2 };
            let foreground = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: samples.subsampling,
                data: vec![0; 3 * bytes],
            };
            native_pixels::overlay_opaque(samples, &foreground, depth, 0, 0)
        };
        let mut output = Cursor::new(Vec::new());
        let (stats, _) = native_lossless::write_mp4_processed(
            &source,
            &mut output,
            &Default::default(),
            &Default::default(),
            None,
            None,
            Some(&mut processor),
        )
        .unwrap();
        assert_eq!(clock, times);
        assert_eq!(stats.video_frames, expected.len() as u64);
        let mut reader =
            NativeReader::software(Cursor::new(output.into_inner()), usize::MAX).unwrap();
        let mut count = 0;
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            let [w, h] = reader.dimensions();
            let samples = VideoGeometry::default().apply(&frame, w, h).unwrap();
            assert_eq!(samples.data, expected[count].0, "{name} frame {count}");
            count += 1;
        }
        assert_eq!(count, expected.len());
    }
}
#[test]
fn processor_failure_is_propagated_before_encoding_the_frame() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let mut output = Cursor::new(Vec::new());
    let mut calls = 0;
    let mut process = |_: &mut GeometryFrame, _: u8, _: u64| {
        calls += 1;
        Err(fvid::Error::Invalid("processor failed".into()))
    };
    assert!(
        native_lossless::write_mp4_processed(
            &source,
            &mut output,
            &Default::default(),
            &Default::default(),
            None,
            None,
            Some(&mut process)
        )
        .is_err()
    );
    assert_eq!(calls, 1);
}
