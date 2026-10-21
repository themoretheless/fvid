#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use fvid::codec::eac3_decoder::transient::apply_window;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-transient-native")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(name)),
            Default::default(),
        )
        .unwrap(),
    )
}
fn packets(stream: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut decoder = stream.make_decoder().unwrap();
    let mut input = Vec::new();
    let mut out = Vec::new();
    let mut windows = Vec::new();
    while let Some(step) = stream.next_step().unwrap() {
        let AudioStep::Encoded(p) = step else {
            panic!("expected encoded access unit")
        };
        input.push((p.pts, p.duration as u64));
        let decoded = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap();
        if input.len() == 1 {
            assert!(decoded.is_none());
        }
        if let Some(p) = decoded {
            windows.push((p.source_pts, p.source_duration));
            let p = stream
                .present_decoded(p.packet, p.source_pts)
                .unwrap()
                .unwrap();
            out.push((p.presentation_time(), p.data));
        }
    }
    while let Some(p) = decoder.finish_packet().unwrap() {
        windows.push((p.source_pts, p.source_duration));
        let p = stream
            .present_decoded(p.packet, p.source_pts)
            .unwrap()
            .unwrap();
        out.push((p.presentation_time(), p.data));
    }
    assert_eq!(
        windows, input,
        "lookahead must retain original access-unit windows"
    );
    assert_eq!(out.len(), 3);
    assert_eq!(
        out.iter().map(|p| p.0).collect::<Vec<_>>(),
        [0, 32, 64].map(Duration::from_millis),
        "delayed output must use its source clock, not lookahead clock"
    );
    assert!(decoder.finish_packet().unwrap().is_none());
    stream.validate_eof().unwrap();
    out
}
#[test]
fn native_transient_player_matches_whole_buffer_correction_and_worker_seek() {
    let control = packets(&mut *open("unity-control.mkv"));
    let samples: Vec<f32> = control
        .iter()
        .flat_map(|(_, b)| {
            b.chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        })
        .collect();
    assert_eq!(samples.len(), 4608 * 2);
    // Unity gain, native stereo: channel correction precedes the identity fold.
    // Evaluate complete windows on a whole-buffer control, then keep the real
    // endpoint. The last event reaches past EOF and must emit only its prefix.
    let mut left = vec![0.0; 1536];
    left.extend(samples.chunks_exact(2).map(|p| p[0]));
    left.resize(1536 + 4872, 0.0);
    for at in [1800, 3336, 4872] {
        apply_window(&mut left, -1536, at, 127).unwrap();
    }
    let expected: Vec<u8> = (0..4608)
        .flat_map(|n| {
            [left[1536 + n].clamp(-1.0, 1.0), samples[n * 2 + 1]]
                .into_iter()
                .flat_map(f32::to_le_bytes)
        })
        .collect();
    let mut stream = open("unity.mkv");
    let got = packets(&mut *stream);
    assert_eq!(
        got.iter()
            .flat_map(|(_, b)| b.iter().copied())
            .collect::<Vec<_>>(),
        expected
    );
    assert_ne!(got, control, "the correction must change sounding PCM");
    assert!(
        got.iter()
            .zip(&control)
            .all(|(g, c)| g.0 == c.0 && g.1.len() == c.1.len())
    );
    stream.rewind();
    assert_eq!(packets(&mut *stream), got);
    let seek = Duration::from_millis(40);
    let wanted = vec![
        (seek, expected[1920 * 8..3072 * 8].to_vec()),
        (Duration::from_millis(64), expected[3072 * 8..].to_vec()),
    ];
    capture::worker_check_with_layout_and_seek_expected(open("unity.mkv"), &got, seek, &wanted, 3);
}
