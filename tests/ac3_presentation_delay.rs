#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
    )
}
#[test]
fn raw_ac3_grouping_does_not_add_a_gap_after_leading_samples_are_trimmed() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-presentation-delay");
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let bytes = std::fs::read(base.join("stream.ac3")).unwrap();
    let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], 48000, 2, 16).unwrap();
    let grouped = engine
        .decode_packet(&bytes, 0, 3072)
        .unwrap()
        .unwrap()
        .packet;
    assert_eq!(grouped.data.len() / 8, 3072 - 256);
    let boundary = Duration::from_nanos(1280 * 1_000_000_000 / 48000);
    let end = Duration::from_nanos(2816 * 1_000_000_000 / 48000);
    let expected = vec![
        (Duration::ZERO, grouped.data[..1280 * 8].to_vec()),
        (boundary, grouped.data[1280 * 8..].to_vec()),
    ];
    let mut source = open(bytes.clone());
    let mut decoder = source.make_decoder().unwrap();
    let mut actual = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        let packet = source
            .present_decoded(f.packet, f.source_pts)
            .unwrap()
            .unwrap();
        actual.push((packet.presentation_time(), packet.data));
    }
    assert_eq!(
        actual
            .iter()
            .flat_map(|(_, p)| p.iter().copied())
            .collect::<Vec<_>>(),
        grouped.data,
        "PCM itself is unchanged by packet grouping"
    );
    if actual != expected || source.duration() != Some(end) {
        eprintln!("presentation mismatch: packet starts{:?}, metadata{:?}; expected starts0/{boundary:?}, end{end:?}",actual.iter().map(|p|p.0).collect::<Vec<_>>(),source.duration());
    }
    capture::worker_check_with_seek(open(bytes), &expected, Some(end));
    assert_eq!(actual, expected);
    assert_eq!(source.duration(), Some(end));
    assert_eq!(source.seek_to(2816), 2816);
    assert!(source.next_packet().unwrap().is_none());
}
