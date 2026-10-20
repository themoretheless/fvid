#![cfg(feature = "player")]
use fvid::playback_native::NativeReader;
use std::{io::Cursor, time::Duration};
#[test]
fn native_video_duration_and_cached_pts_keep_integer_clock_precision() {
    let media = include_bytes!("fixtures/playback-errors/native-video-clock/clock.mp4");
    let mut reader = NativeReader::software(Cursor::new(media.as_slice()), 32 << 20).unwrap();
    assert_eq!(reader.duration(), Some(Duration::from_secs(20_000_000_001)));
    let cache = reader.cache_packets();
    assert_eq!(cache.len(), 12);
    let time = |ticks: u64| Duration::new(ticks / 7, ((ticks % 7) * 1_000_000_000 / 7) as u32);
    for (i, &(_, _, start, end)) in cache.iter().enumerate() {
        assert_eq!(start, time(i as u64 * 2_000_000_001), "packet {i}");
        assert_eq!(end, time((i as u64 + 1) * 2_000_000_001), "packet {i}");
        assert!(reader.read_frame_raw().unwrap().is_some());
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    reader.rewind().unwrap();
    assert_eq!(reader.cache_packets(), cache);
}
