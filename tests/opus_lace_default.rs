#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn read(bytes: Vec<u8>) -> (Vec<(Duration, Vec<u8>)>, Option<Duration>) {
    let mut source =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut packets = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(p) = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            assert!(
                p.data
                    .chunks_exact(4)
                    .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
            );
            packets.push((p.presentation_time(), p.data));
        }
    }
    source.validate_eof().unwrap();
    (packets, source.duration())
}
#[test]
fn opus_packet_toc_timing_overrides_track_default_for_variable_laces() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/opus-lace-default");
    let mut wrong = Vec::new();
    for mode in ["xiph", "ebml"] {
        let control_bytes = std::fs::read(base.join(format!("{mode}-control.mkv"))).unwrap();
        let mut owned_control = Vec::new();
        let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&control_bytes),
            &mut owned_control,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 4320);
        let (expected, end) = read(control_bytes);
        assert_eq!(end, Some(Duration::from_millis(90)));
        assert_eq!(
            expected.iter().map(|p| p.0).collect::<Vec<_>>(),
            [0, 20, 60, 70].map(Duration::from_millis)
        );
        assert_eq!(
            expected.iter().map(|p| p.1.len() / 4).collect::<Vec<_>>(),
            vec![960, 1920, 480, 960]
        );
        let unsupported = std::fs::read(base.join(format!("{mode}-unsupported-head.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(unsupported), 32 << 20)
                .unwrap();
        for _ in 0..5 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none()); // Video-only indexability, not audio acceptance.
        for label in ["default", "default-short", "default-long"] {
            let bytes = std::fs::read(base.join(format!("{mode}-{label}.mkv"))).unwrap();
            let mut video =
                fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                    .unwrap();
            for _ in 0..5 {
                assert!(video.read_frame_raw().unwrap().is_some());
            }
            assert!(video.read_frame_raw().unwrap().is_none());
            let mut owned = Vec::new();
            let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                Cursor::new(&bytes),
                &mut owned,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, 4320);
            assert_eq!(owned, owned_control, "{mode}/{label} owned timeline");
            let (actual, end) = read(bytes.clone());
            assert_eq!(
                actual.iter().flat_map(|p| p.1.iter()).collect::<Vec<_>>(),
                expected.iter().flat_map(|p| p.1.iter()).collect::<Vec<_>>(),
                "native decoded data control"
            );
            if actual != expected || end != Some(Duration::from_millis(90)) {
                wrong.push(format!(
                    "{mode}: times {:?} vs {:?}, end{end:?}",
                    actual.iter().map(|p| p.0).collect::<Vec<_>>(),
                    expected.iter().map(|p| p.0).collect::<Vec<_>>()
                ));
                continue;
            }
            let source = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            capture::worker_check_with_seek(Box::new(source), &expected, Some(Duration::ZERO));
        }
    }
    assert!(
        wrong.is_empty(),
        "track default replaced variable Opus lace timing: {wrong:?}"
    );
}
