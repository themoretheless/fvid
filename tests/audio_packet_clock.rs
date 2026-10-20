#![cfg(feature = "player")]
use fvid::audio::{AudioPacket, AudioStream};
use std::{io::Cursor, time::Duration};
#[test]
fn decoded_packet_clock_preserves_seconds_beyond_u64_nanoseconds() {
    let media = include_bytes!("fixtures/playback-errors/audio-packet-clock/clock.mkv");
    let mut video = fvid::playback_native::NativeReader::software(Cursor::new(media.as_slice()),32<<20).unwrap();
    for _ in 0..3 { assert!(video.read_frame_raw().unwrap().is_some()); }
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut source = fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media.as_slice()),Default::default()).unwrap();
    let encoded = source.next_packet().unwrap().unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let decoded = decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
    let packet=source.present_decoded(decoded.packet,decoded.source_pts).unwrap().unwrap();
    assert_eq!(packet.data.as_slice(),include_bytes!("fixtures/playback-errors/audio-packet-clock/expected-source.f32le"));
    assert_eq!(packet.presentation_time(),Duration::ZERO);
    // Timestamp injection preserves the valid decoded payload. This qualifies
    // the public packet conversion, not acceptance of a huge container timeline.
    for (pts,num,den,expected) in [
        (20_000_000_001,1,1,Duration::from_secs(20_000_000_001)),
        (40_000_000_001,1,2,Duration::new(20_000_000_000,500_000_000)),
        (u64::MAX,1,1,Duration::from_secs(u64::MAX)),
        (u64::MAX,u32::MAX,1,Duration::MAX),
        (1,1,3,Duration::from_nanos(333_333_333)),
        (1,1,0,Duration::ZERO),
    ] {
        let changed=AudioPacket{data:packet.data.clone(),pts,timebase_num:num,timebase_den:den};
        assert_eq!(changed.presentation_time(),expected,"pts={pts},timebase={num}/{den}");
    }
}
