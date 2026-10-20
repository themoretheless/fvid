#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,time::Duration};
#[test]
fn stored_matroska_timestamp_matches_exact_default_stream_clock() {
    let media=include_bytes!("fixtures/playback-errors/audio-stream-clock/clock.mkv");
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(media.as_slice()),32<<20).unwrap();
    for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media.as_slice()),Default::default()).unwrap();
    let encoded=source.next_packet().unwrap().unwrap();
    assert_eq!(encoded.pts,9_000_000_000_001_000_000);
    let mut decoder=source.make_decoder().unwrap();
    let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
    let packet=source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap();
    assert_eq!(packet.data.as_slice(),include_bytes!("fixtures/playback-errors/audio-stream-clock/expected-source.f32le"));
    let expected=Duration::new(9_000_000_000,1_000_000);
    assert_eq!(packet.presentation_time(),expected);
    assert_eq!(source.time_of(encoded.pts),expected,"seek anchor loses stored timestamp precision");
    for pts in [-1,0,1,999_999_999,1_000_000_000,1_000_000_001,i64::MAX] {
        assert_eq!(source.time_of(pts),Duration::from_nanos(pts.max(0) as u64));
    }
}
