use fvid::container::mp4::{Limits, Mp4Reader};
use std::{
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-signed-offset")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[derive(Default)]
struct Stats {
    bytes: AtomicUsize,
    calls: AtomicUsize,
    max_request: AtomicUsize,
}
struct Sparse {
    size: u64,
    at: u64,
    segments: Vec<(u64, Vec<u8>)>,
    stats: Arc<Stats>,
}
impl Sparse {
    fn file(n: &str) -> Self {
        let b = bytes(n);
        Self {
            size: b.len() as u64,
            at: 0,
            segments: vec![(0, b)],
            stats: Arc::default(),
        }
    }
    fn virtual_file(key: &str) -> Self {
        let m = manifest();
        let c = m["virtual"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == key)
            .unwrap();
        let segments: Vec<_> = c["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["offset"].as_u64().unwrap(),
                    bytes(s["file"].as_str().unwrap()),
                )
            })
            .collect();
        let size = c["size"].as_u64().unwrap();
        assert!(size > 1 << 31);
        assert!(segments.iter().map(|(_, b)| b.len()).sum::<usize>() < 32 << 10);
        for pair in segments.windows(2) {
            assert!(pair[0].0 + pair[0].1.len() as u64 <= pair[1].0);
        }
        assert_eq!(
            segments.last().unwrap().0 + segments.last().unwrap().1.len() as u64,
            size
        );
        Self {
            size,
            at: 0,
            segments,
            stats: Arc::default(),
        }
    }
}
impl Read for Sparse {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stats.calls.fetch_add(1, Ordering::Relaxed);
        self.stats
            .max_request
            .fetch_max(out.len(), Ordering::Relaxed);
        assert!(
            out.len() <= 64 << 10,
            "unexpected giant metadata/sample read"
        );
        let n = out.len().min(self.size.saturating_sub(self.at) as usize);
        out[..n].fill(0);
        let end = self.at + n as u64;
        for (start, b) in &self.segments {
            let lo = self.at.max(*start);
            let hi = end.min(*start + b.len() as u64);
            if lo < hi {
                out[(lo - self.at) as usize..(hi - self.at) as usize]
                    .copy_from_slice(&b[(lo - start) as usize..(hi - start) as usize]);
            }
        }
        self.at = end;
        self.stats.bytes.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}
impl Seek for Sparse {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.at) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.size) + i128::from(n),
        };
        self.at = u64::try_from(at)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid virtual seek"))?;
        Ok(self.at)
    }
}
fn bounded(stats: &Stats, cap: usize) {
    assert!(
        stats.bytes.load(Ordering::Relaxed) < cap,
        "unexpected actual read volume"
    );
    assert!(stats.max_request.load(Ordering::Relaxed) <= 64 << 10);
    assert!(stats.calls.load(Ordering::Relaxed) < 256);
}
fn indexed(s: Sparse) {
    let stats = s.stats.clone();
    let size = s.size;
    let mut p = Mp4Reader::open(
        s,
        Limits {
            samples: 3,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(p.tracks().len(), 1);
    assert_eq!((p.tracks()[0].timescale, p.tracks()[0].duration), (50, 3));
    assert_eq!(p.tracks()[0].samples.len(), 3);
    let mut data = Vec::new();
    for i in 0..3 {
        let v = p.tracks()[0].samples.get(i).unwrap();
        assert_eq!(
            (v.dts, v.pts, v.duration, v.sync),
            (i as u64, i as i64, 1, true)
        );
        p.read_packet(0, i, &mut data).unwrap();
        assert_eq!(data, bytes("frame.vp9"));
    }
    bounded(&stats, 64 << 10);
    println!(
        "logical {size} bytes:index and3 exact packets read{} actual bytes in{} calls",
        stats.bytes.load(Ordering::Relaxed),
        stats.calls.load(Ordering::Relaxed)
    );
}
fn refusal(s: Sparse) {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Mp4Reader::open(s, Default::default())
    }));
    assert!(caught.is_ok(), "MP4 offset arithmetic must not unwind");
    let e = match caught.unwrap() {
        Ok(_) => panic!("underflowing offset admitted"),
        Err(e) => e,
    };
    assert!(matches!(e, fvid::Error::Invalid(_)));
    assert!(
        e.to_string()
            .contains("MP4 fragment data offset before its base"),
        "{e}"
    );
}
#[test]
fn ordinary_negative_and_adjacent_boundary_controls_keep_exact_packets() {
    indexed(Sparse::file("positive.mp4"));
    indexed(Sparse::file("negative.mp4"));
    indexed(Sparse::virtual_file("valid-adjacent"));
    refusal(Sparse::file("adjacent-underflow.mp4"));
}
#[test]
fn the_entire_signed_range_resolves_or_refuses_without_panicking() {
    refusal(Sparse::file("min-underflow.mp4"));
    indexed(Sparse::virtual_file("valid-min"));
}
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::playback_native::{NativeReader, RawFrame};
    use std::time::Duration;
    fn frame(f: RawFrame) {
        match &f {
            RawFrame::Planar(p) => {
                assert_eq!(p.depth, 8);
                assert_eq!((p.frame.width, p.frame.height), (16, 16));
                assert_eq!(p.frame.data, vec![128; 384]);
            }
            _ => panic!("native packed VP9 expected"),
        };
        assert_eq!(f.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
    }
    fn play(input: Sparse) {
        let stats = input.stats.clone();
        let size = input.size;
        let mut s =
            NativeReader::software(BufReader::with_capacity(1024, input), 32 << 20).unwrap();
        assert_eq!(s.duration(), Some(Duration::from_millis(60)));
        for i in 0..3 {
            frame(s.read_frame_raw().unwrap().unwrap());
            assert_eq!(s.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(s.read_frame_raw().unwrap().is_none());
        assert!(s.read_frame_raw().unwrap().is_none());
        s.rewind().unwrap();
        frame(s.read_frame_raw().unwrap().unwrap());
        assert_eq!(s.frame_interval(), Some((0, 1, 50)));
        for ms in [0, 19, 20, 39, 40, 59, 60, 80] {
            frame(s.seek_raw(Duration::from_millis(ms)).unwrap().unwrap());
            let i = u128::from(ms.min(59) / 20);
            assert_eq!(s.frame_interval(), Some((i, i + 1, 50)));
            assert_eq!(s.current_pts(), Some((i as i64, 50)));
            if ms >= 60 {
                assert!(s.read_frame_raw().unwrap().is_none());
                assert!(s.read_frame_raw().unwrap().is_none());
            }
        }
        bounded(&stats, 128 << 10);
        println!(
            "logical {size} bytes:native PCM-free VP9 full playback/navigation read{} actual bytes",
            stats.bytes.load(Ordering::Relaxed)
        );
    }
    #[test]
    fn ordinary_and_adjacent_offsets_keep_analytic_playback_and_public_navigation() {
        play(Sparse::file("positive.mp4"));
        play(Sparse::file("negative.mp4"));
        play(Sparse::virtual_file("valid-adjacent"));
    }
    #[test]
    fn valid_min_displacement_plays_without_gigabyte_reads_and_corruption_refuses() {
        play(Sparse::virtual_file("valid-min"));
        let r = std::panic::catch_unwind(|| {
            NativeReader::software(
                BufReader::with_capacity(1024, Sparse::file("min-underflow.mp4")),
                32 << 20,
            )
        });
        assert!(
            r.is_ok(),
            "corrupt offset reached arithmetic panic in native open"
        );
        let e = match r.unwrap() {
            Ok(_) => panic!("corrupt offset admitted"),
            Err(e) => e,
        };
        assert!(matches!(e, fvid::Error::Invalid(_)));
        assert!(
            e.to_string()
                .contains("MP4 fragment data offset before its base"),
            "{e}"
        );
    }
}
