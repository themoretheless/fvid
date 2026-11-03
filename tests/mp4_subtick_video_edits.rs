use fvid::playback_native::NativeReader;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-subtick-video-edits")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn authored_source_and_integral_edit_decode_the_exact_three_known_pictures() {
    let names = std::iter::once("unedited.mp4".to_owned())
        .chain(
            cases()
                .iter()
                .map(|c| c["file"].as_str().unwrap().to_owned()),
        )
        .chain(std::iter::once("below-nanosecond.mp4".to_owned()))
        .collect::<Vec<_>>();
    for name in names {
        let manifest = cases();
        let case = manifest
            .iter()
            .find(|c| c["file"].as_str() == Some(name.as_str()));
        let parsed = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(&name)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(parsed.tracks()[0].timescale, 50);
        assert_eq!(parsed.tracks()[0].samples.len(), 3);
        let mut raw = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(&name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let frame = raw.read_frame().unwrap().unwrap();
            assert_eq!(
                (frame.presentation_time.ticks, frame.duration.ticks),
                (
                    case.map_or(i, |c| c["raw_pts"][i as usize].as_i64().unwrap()),
                    1
                )
            );
            let pixels = frame
                .picture
                .y
                .iter()
                .chain(&frame.picture.cb)
                .chain(&frame.picture.cr)
                .flat_map(|n| n.to_le_bytes())
                .collect::<Vec<_>>();
            let picture = case.map_or(i as u64, |c| c["raw_order"][i as usize].as_u64().unwrap());
            assert_eq!(pixels, bytes(&format!("frame-{picture}.yuv16")), "{name}");
        }
        assert!(raw.read_frame().unwrap().is_none());
    }
    let mut reader =
        NativeReader::software(Cursor::new(bytes("control-20.mp4")), 32 << 20).unwrap();
    assert_eq!(reader.duration(), Some(Duration::from_millis(20)));
    assert!(reader.read_frame().unwrap());
    let (start, end, scale) = reader.frame_interval().unwrap();
    assert_eq!(start, 0);
    assert_eq!(end * 1000, 20 * u128::from(scale));
    assert!(!reader.read_frame().unwrap());
}
#[test]
fn positive_fractional_video_edits_keep_exact_movie_duration_and_frame_intervals() {
    let mut source = NativeReader::software(Cursor::new(bytes("unedited.mp4")), 32 << 20).unwrap();
    let mut colors = Vec::new();
    for _ in 0..3 {
        assert!(source.read_frame().unwrap());
        colors.push(source.rgb().to_vec());
    }
    let mut wrong = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut reader = match NativeReader::software(Cursor::new(bytes(name)), 32 << 20) {
            Ok(r) => r,
            Err(e) => {
                wrong.push(format!("{name}: refused {e}"));
                continue;
            }
        };
        let expected = Duration::from_nanos(c["duration_ns"].as_u64().unwrap());
        if reader.duration() != Some(expected) {
            wrong.push(format!(
                "{name}: duration {:?}, expected{expected:?}",
                reader.duration()
            ));
        }
        for frame in c["frames"].as_array().unwrap() {
            if !reader.read_frame().unwrap() {
                wrong.push(format!("{name}: early EOF"));
                break;
            }
            assert_eq!(
                reader.rgb(),
                colors[frame["source"].as_u64().unwrap() as usize],
                "{name}: source picture"
            );
            let (start, end, scale) = reader.frame_interval().unwrap();
            let a = frame["start_ns"].as_u64().unwrap();
            let b = frame["end_ns"].as_u64().unwrap();
            assert_eq!(
                reader.frame_period(),
                Duration::from_nanos(b - a),
                "{name}: exact frame period"
            );
            let (pts, clock) = reader.current_pts().unwrap();
            assert_eq!(
                u128::try_from(pts).unwrap() * 1_000_000_000 / u128::from(clock),
                u128::from(a),
                "{name}: presentation timestamp"
            );
            if start * 1_000_000_000 / u128::from(scale) != u128::from(a)
                || end * 1_000_000_000 / u128::from(scale) != u128::from(b)
            {
                wrong.push(format!(
                    "{name}: interval {start}..{end}/{scale}, expected{a}..{b}ns"
                ));
            }
        }
        if reader.read_frame().unwrap() {
            wrong.push(format!("{name}: unexpected extra frame"));
        }
    }
    assert!(
        wrong.is_empty(),
        "fractional movie-clock edits changed: {wrong:#?}"
    );
}

#[test]
fn exact_edit_seek_rewind_cache_and_resource_limits_keep_the_same_pictures() {
    let mut plain = NativeReader::software(Cursor::new(bytes("unedited.mp4")), 32 << 20).unwrap();
    let mut colors = Vec::new();
    for _ in 0..3 {
        assert!(plain.read_frame().unwrap());
        colors.push(plain.rgb().to_vec());
    }
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let mut reader = NativeReader::software(Cursor::new(data.clone()), 32 << 20).unwrap();
        let parsed = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(data.clone()),
            Default::default(),
        )
        .unwrap();
        let frames = c["frames"].as_array().unwrap();
        let mut expected = frames
            .iter()
            .map(|f| {
                let s = parsed.tracks()[0]
                    .samples
                    .get(f["source"].as_u64().unwrap() as usize)
                    .unwrap();
                (
                    s.offset,
                    s.offset + u64::from(s.size),
                    Duration::from_nanos(f["start_ns"].as_u64().unwrap()),
                    Duration::from_nanos(f["end_ns"].as_u64().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        let mut cache = reader.cache_packets();
        cache.sort();
        expected.sort();
        assert_eq!(cache, expected, "{name}: exact cache clock");
        for f in frames {
            let a = f["start_ns"].as_u64().unwrap();
            let b = f["end_ns"].as_u64().unwrap();
            reader.seek(Duration::from_nanos(a + (b - a) / 2)).unwrap();
            assert_eq!(
                reader.rgb(),
                colors[f["source"].as_u64().unwrap() as usize],
                "{name}: seek picture"
            );
            let (start, end, scale) = reader.frame_interval().unwrap();
            assert_eq!(start * 1_000_000_000 / u128::from(scale), u128::from(a));
            assert_eq!(end * 1_000_000_000 / u128::from(scale), u128::from(b));
        }
        reader.seek(Duration::MAX).unwrap();
        assert_eq!(
            reader.rgb(),
            colors[frames.last().unwrap()["source"].as_u64().unwrap() as usize]
        );
        assert!(
            !reader.read_frame().unwrap(),
            "{name}: EOF after terminal seek"
        );
        reader.rewind().unwrap();
        assert!(reader.current_pts().is_none());
        for f in frames {
            assert!(reader.read_frame().unwrap());
            assert_eq!(reader.rgb(), colors[f["source"].as_u64().unwrap() as usize]);
        }
        assert!(!reader.read_frame().unwrap());
        assert!(
            NativeReader::software(Cursor::new(data), 1).is_err(),
            "{name}: metadata quota bypass"
        );
    }
}

#[test]
fn a_positive_but_subnanosecond_edit_is_a_precision_refusal_not_corrupt_media() {
    let error = NativeReader::software(Cursor::new(bytes("below-nanosecond.mp4")), 32 << 20)
        .err()
        .expect("unrepresentable frame interval was expanded");
    assert!(matches!(error, fvid::Error::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("nanosecond playback precision"));
}

#[test]
fn a_fractional_late_seek_keeps_indexed_packet_io_instead_of_replaying_the_prefix() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Counted {
        cursor: Cursor<Vec<u8>>,
        bytes: Arc<AtomicUsize>,
    }
    impl std::io::Read for Counted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = std::io::Read::read(&mut self.cursor, out)?;
            self.bytes.fetch_add(n, Ordering::Relaxed);
            Ok(n)
        }
    }
    impl std::io::BufRead for Counted {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            std::io::BufRead::fill_buf(&mut self.cursor)
        }
        fn consume(&mut self, n: usize) {
            std::io::BufRead::consume(&mut self.cursor, n)
        }
    }
    impl std::io::Seek for Counted {
        fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.cursor, from)
        }
    }
    let data = bytes("seek-cost.mp4");
    let parsed = fvid::container::mp4::Mp4Reader::open_indexed(
        Cursor::new(data.clone()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(parsed.tracks()[0].samples.len(), 64);
    let packet = parsed.tracks()[0].samples.get(50).unwrap().size as usize;
    let count = Arc::new(AtomicUsize::new(0));
    let mut reader = NativeReader::software(
        Counted {
            cursor: Cursor::new(data),
            bytes: count.clone(),
        },
        32 << 20,
    )
    .unwrap();
    assert_eq!(reader.duration(), Some(Duration::from_millis(1261)));
    count.store(0, Ordering::Relaxed);
    reader.seek(Duration::from_millis(1001)).unwrap();
    assert!(
        count.load(Ordering::Relaxed) <= 4 * packet,
        "indexed seek replayed prefix: {} bytes, packet {packet}",
        count.load(Ordering::Relaxed)
    );
    let (start, end, scale) = reader.frame_interval().unwrap();
    assert_eq!(start * 1000 / u128::from(scale), 1000);
    assert_eq!(end * 1000 / u128::from(scale), 1020);
    let mut reference =
        NativeReader::software(Cursor::new(bytes("unedited.mp4")), 32 << 20).unwrap();
    for _ in 0..3 {
        assert!(reference.read_frame().unwrap());
    }
    assert_eq!(reader.rgb(), reference.rgb());
}
