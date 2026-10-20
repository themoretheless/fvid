//! A software VP9 seek must decode long-GOP pre-roll and land on exactly the
//! same displayed picture as sequential playback. No FFmpeg/network at runtime.
use fvid::playback_native::NativeReader;
use std::{io::Cursor, time::Duration};

#[test]
fn software_long_gop_seek_matches_sequential_pictures_forward_and_backward() {
    let data = include_bytes!("fixtures/playback-errors/vp9-long-gop-seek.webm");
    let mut reader = NativeReader::software(Cursor::new(data), 256 << 20).unwrap();
    let targets = [0.5, 3.2, 6.6, 9.9];
    let mut pictures = Vec::new();
    for seconds in targets {
        let target = Duration::from_secs_f64(seconds);
        loop {
            assert!(reader.read_frame().unwrap());
            let (start, end, scale) = reader.frame_interval().unwrap();
            let ticks = target.as_nanos() * u128::from(scale);
            if start * 1_000_000_000 <= ticks && ticks < end * 1_000_000_000 {
                pictures.push((reader.frame_interval().unwrap(), reader.rgb().to_vec()));
                break;
            }
        }
    }
    if let NativeReader::Webm(webm) = &mut reader {
        assert_eq!(webm.seek_to_sync(3_200_000_000).unwrap(), 0,
            "explicit sync seeking must still land on the real keyframe");
    }
    let before = reader.seek_cache_stats().unwrap().2;
    for index in [2, 0, 1, 3, 2, 0] {
        let target = Duration::from_secs_f64(targets[index]);
        let raw = reader.seek_raw(target).unwrap().expect("seek target picture");
        assert_eq!(reader.frame_interval().unwrap(), pictures[index].0);
        assert_eq!(raw.into_rgb(256 << 20).unwrap(), pictures[index].1);
        // Refresh RGB through the public seek path, then compare every pixel.
        reader.seek(target).unwrap();
        assert_eq!(reader.rgb(), pictures[index].1);
    }
    assert!(reader.seek_cache_stats().unwrap().2 >= before + 6,
        "loaded long-GOP seeks must restore prepared decoder checkpoints");
}

#[test]
fn background_preparation_makes_unvisited_gops_available_for_seek() {
    let data = include_bytes!("fixtures/playback-errors/vp9-long-gop-seek.webm");
    let path = std::env::temp_dir().join(format!("fvid-vp9-seek-warm-{}.webm", std::process::id()));
    std::fs::write(&path, data).unwrap();
    let mut reader = NativeReader::software(std::io::BufReader::new(std::fs::File::open(&path).unwrap()), 256 << 20).unwrap();
    assert!(reader.read_frame_raw().unwrap().is_some());
    // The real player already holds its first picture before preparation starts.
    // Leave this reader idle while the worker must fill the whole remaining chain.
    reader.warm_seeking(&path);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !reader.seek_frame_cache_complete() {
        assert!(std::time::Instant::now() < until, "checkpoint preparation did not progress");
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = reader.seek_cache_stats().unwrap().2;
    assert!(reader.seek_raw(Duration::from_secs_f64(5.0)).unwrap().is_some());
    assert!(reader.seek_cache_stats().unwrap().2 > before);
    let (start, end, scale) = reader.frame_interval().unwrap();
    let target = 5_000_000_000u128 * u128::from(scale);
    assert!(start * 1_000_000_000 <= target && target < end * 1_000_000_000);
    let mut reference = NativeReader::software(Cursor::new(data), 256 << 20).unwrap();
    for seconds in [0.5, 3.2, 6.6, 9.9] {
        let target = Duration::from_secs_f64(seconds);
        reference.seek(target).unwrap();
        let raw = reader.seek_raw(target).unwrap().unwrap();
        assert_eq!(reader.frame_interval(), reference.frame_interval());
        assert_eq!(raw.into_rgb(256 << 20).unwrap(), reference.rgb());
    }
    drop(reader);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn ready_frames_continue_after_seek_and_mix_raw_and_rgb_without_skips() {
    let data = include_bytes!("fixtures/playback-errors/vp9-long-gop-seek.webm");
    let mut reader = NativeReader::software(Cursor::new(data), 256 << 20).unwrap();
    let mut expected = Vec::new();
    while let Some(raw) = reader.read_frame_raw().unwrap() {
        expected.push((reader.frame_interval().unwrap(), raw.into_rgb(256 << 20).unwrap()));
    }
    assert!(reader.seek_frame_cache_complete());
    assert_eq!(reader.seek_frame_cache_count(), expected.len());
    let ranges = reader.seek_cache_status().unwrap().ranges(reader.duration().unwrap());
    assert_eq!(ranges, vec![[0.0, 1.0]], "ready frames must cover the blue buffer strip");
    let target = Duration::from_secs_f64(3.2);
    let index = expected.iter().position(|((a,b,scale),_)|
        *a * 1_000_000_000 <= target.as_nanos() * u128::from(*scale)
        && target.as_nanos() * u128::from(*scale) < *b * 1_000_000_000).unwrap();
    let raw = reader.seek_raw(target).unwrap().unwrap();
    assert_eq!(raw.into_rgb(256 << 20).unwrap(), expected[index].1);
    for (offset, (interval, pixels)) in expected.iter().enumerate().skip(index + 1) {
        if offset % 2 == 0 {
            assert!(reader.read_frame().unwrap());
            assert_eq!(reader.rgb(), pixels);
        } else {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            assert_eq!(&raw.into_rgb(256 << 20).unwrap(), pixels);
        }
        assert_eq!(&reader.frame_interval().unwrap(), interval);
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    assert!(reader.read_frame_raw().unwrap().is_none());
    reader.rewind().unwrap();
    assert!(reader.read_frame().unwrap());
    assert_eq!(reader.rgb(), expected[0].1);
}

#[test]
fn eviction_falls_back_to_decoding_without_skipping_uncached_pictures() {
    let data = include_bytes!("fixtures/playback-errors/vp9-long-gop-seek.webm");
    let mut reference = NativeReader::software(Cursor::new(data), 256 << 20).unwrap();
    let mut expected = Vec::new();
    while let Some(raw) = reference.read_frame_raw().unwrap() {
        expected.push((reference.frame_interval().unwrap(), raw.into_rgb(256 << 20).unwrap()));
    }
    let mut reader = NativeReader::software(Cursor::new(data), 2 << 20).unwrap();
    while reader.read_frame_raw().unwrap().is_some() {}
    assert!(!reader.seek_frame_cache_complete(), "small budget must evict earlier pictures");
    let ranges = reader.seek_cache_status().unwrap().ranges(reader.duration().unwrap());
    assert!(ranges.first().is_some_and(|range| range[0] > 0.0), "evicted pictures must disappear from the strip");
    for seconds in [0.5, 6.6, 3.2, 9.9] {
        let target = Duration::from_secs_f64(seconds);
        let at = expected.iter().position(|((a,b,scale),_)|
            *a * 1_000_000_000 <= target.as_nanos() * u128::from(*scale)
            && target.as_nanos() * u128::from(*scale) < *b * 1_000_000_000).unwrap();
        let raw = reader.seek_raw(target).unwrap().unwrap();
        assert_eq!(raw.into_rgb(2 << 20).unwrap(), expected[at].1);
        assert_eq!(reader.frame_interval().unwrap(), expected[at].0);
        for (interval, pixels) in expected.iter().skip(at + 1).take(5) {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            assert_eq!(&raw.into_rgb(2 << 20).unwrap(), pixels);
            assert_eq!(&reader.frame_interval().unwrap(), interval);
        }
    }
}
