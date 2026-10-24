use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/video-readers")
}

#[test]
fn matroska_reordered_packed_frames_enforce_queue_budget_and_inband_hevc_opens() {
    let reordered_data = fs::read(root().join("avc-reordered-packed-budget.mkv")).unwrap();

    // With an ample budget, both reordered AVC 4:4:4 PackedPlanar frames decode in presentation order.
    let mut full = fvid::playback_webm::WebmVideoReader::open_software(
        Cursor::new(&reordered_data),
        16 << 20,
    )
    .unwrap();
    let mut count = 0;
    while full.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 6);

    // With a tight budget (e.g. 24000 bytes -> queue_budget = 9000 bytes, rgb_budget = 6000 bytes),
    // six queued 32x16 4:4:4 8-bit PackedPlanar frames (6 * 1536 = 9216 bytes + HdrMetadata)
    // exceed queue_budget and must be rejected as "Matroska reordered frames exceed memory budget".
    let mut tight = fvid::playback_webm::WebmVideoReader::open_software(
        Cursor::new(&reordered_data),
        24000,
    )
    .unwrap();
    let mut budget_err = None;
    while let Some(res) = tight.read_frame_raw().transpose() {
        match res {
            Ok(_) => {}
            Err(err) => {
                budget_err = Some(err);
                break;
            }
        }
    }
    let err = budget_err.expect("tight queue_budget must reject queued PackedPlanar frames");
    assert!(
        err.to_string()
            .contains("Matroska reordered frames exceed memory budget"),
        "unexpected error: {err}"
    );

    // Matroska HEVC with empty hvcC array and in-band VPS/SPS/PPS arriving in packet 1
    // (after an AUD-only packet 0) must open in software mode, safely report default
    // bitstream_colour() before packet 1 is decoded, and decode the in-band frame.
    let inband_data = fs::read(root().join("hevc-inband-deferred-sps.mkv")).unwrap();
    let mut inband = fvid::playback_webm::WebmVideoReader::open_software(
        Cursor::new(&inband_data),
        16 << 20,
    )
    .unwrap();
    assert_eq!(
        inband.bitstream_colour(),
        fvid::color::hdr::ColourDescription::default()
    );
    let raw = inband
        .read_frame_raw()
        .unwrap()
        .expect("in-band HEVC frame in packet 1 must decode");
    assert_eq!(inband.dimensions(), [64, 64]);
    drop(raw);
}

#[test]
fn mp4_native_reader_current_pts_is_none_before_first_frame_and_after_rewind() {
    let mp4_data = fs::read(root().join("hevc-minimal-pts.mp4")).unwrap();
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(&mp4_data),
        16 << 20,
    )
    .unwrap();

    // Before any frame is read, current_pts() must return None (matching frame_interval()).
    assert_eq!(reader.frame_interval(), None);
    assert_eq!(reader.current_pts(), None);

    assert!(reader.read_frame().unwrap());
    assert!(reader.current_pts().is_some());

    // After rewind(), no frame has been read yet so current_pts() must again be None.
    reader.rewind().unwrap();
    assert_eq!(reader.frame_interval(), None);
    assert_eq!(reader.current_pts(), None);
}
