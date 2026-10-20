#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/xm_pan_reference.rs"]
mod pan_reference;
fn base() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-set-panning")
}
fn open(bytes: Vec<u8>) -> fvid::playback_xm::XmAudioReader {
    fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap()
}
fn render(bytes: Vec<u8>, channels: u16) -> (Vec<(Duration, Vec<u8>)>, u32) {
    let mut source = open(bytes);
    let mut decoder = fvid::codec::xm_decoder::XmDecoder::new(
        source.extra_data(),
        source.sample_rate(),
        channels,
    )
    .unwrap();
    let mut out = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        out.push((f.packet.presentation_time(), f.packet.data));
    }
    (out, decoder.ignored().effects)
}
#[test]
fn native_xm_set_panning_is_supported_and_changes_stereo() {
    let base = base();
    let (actual, ignored) = render(std::fs::read(base.join("pan-0.xm")).unwrap(), 2);
    let (without, _) = render(std::fs::read(base.join("pan-0-without.xm")).unwrap(), 2);
    assert!(
        actual != without && ignored == 0,
        "native 8xx changes stereo and is supported"
    );
}
#[test]
fn native_xm_set_panning_acceptance_matches_instrument_panning_without_phase_reset() {
    let base = base();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();
        let reference = open(control.clone());
        assert_eq!(open(bytes.clone()).duration(), reference.duration());
        let mut stereo = Vec::new();
        let mut static_stereo = Vec::new();
        for channels in [1, 2] {
            let (expected, static_reference) =
                pan_reference::expected(&base, case, channels, render);
            let (actual, ignored) = render(bytes.clone(), channels);
            if ignored != 0 {
                failures.push(format!("{name}/{channels}:8xxstill ignored"));
            }
            if actual != expected {
                failures.push(format!(
                    "{name}/{channels}:8xxdiffers from independent instrument pan"
                ));
            }
            if channels == 2 {
                stereo = expected;
                static_stereo = static_reference;
            }
        }
        capture::worker_check_with_seek(
            Box::new(open(control)),
            &static_stereo,
            reference.duration(),
        );
        let row = case["seek"].as_u64().unwrap() as usize;
        for (target, suffix) in [
            (Duration::from_millis(row as u64 * 120), &stereo[row..]),
            (reference.duration().unwrap(), &stereo[stereo.len()..]),
        ] {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek_expected(
                    Box::new(open(bytes.clone())),
                    &stereo,
                    target,
                    suffix,
                )
            }))
            .is_err()
            {
                failures.push(format!("{name}: actual8xxplayback/seek differs"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
