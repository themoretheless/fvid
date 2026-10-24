use fvid_media::decode_video;
use fvid_media::owned_mp4::{Limits, Mp4Reader};
use fvid_media::owned_mp4_probe::probe_mp4;
use std::{
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn fixture_path(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(dir)
        .join(name)
}

fn bytes(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(dir, name)).unwrap()
}

fn manifest(dir: &str) -> serde_json::Value {
    serde_json::from_slice(&bytes(dir, "manifest.json")).unwrap()
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
    fn virtual_file(dir: &str, key: &str) -> Self {
        let m = manifest(dir);
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
                    bytes(dir, s["file"].as_str().unwrap()),
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
        assert!(
            self.stats.bytes.load(Ordering::Relaxed) <= 128 << 10,
            "sparse fixture read budget exceeded"
        );
        Ok(n)
    }
}

impl Seek for Sparse {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(x) => i128::from(x),
            SeekFrom::End(x) => i128::from(self.size) + i128::from(x),
            SeekFrom::Current(x) => i128::from(self.at) + i128::from(x),
        };
        let next = u64::try_from(next).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek before start or overflow")
        })?;
        self.at = next;
        Ok(self.at)
    }
}

const ACCEPTANCE: bool = option_env!("FVID_BATCH_178_ACCEPTANCE").is_some();

fn verify_owned_packets(mut reader: Mp4Reader<impl Read + Seek>, label: &str, packet: &[u8]) {
    assert_eq!(reader.tracks().len(), 1, "{label}");
    assert_eq!(reader.tracks()[0].codec, *b"vp09", "{label}");
    assert_eq!(
        (reader.tracks()[0].timescale, reader.tracks()[0].duration),
        (50, 3),
        "{label}"
    );
    assert_eq!(reader.tracks()[0].samples.len(), 3, "{label}");
    for i in 0..3 {
        let s = reader.tracks()[0].samples.get(i).unwrap();
        assert_eq!(
            (s.dts, s.pts, s.duration, s.sync),
            (i as u64, i as i64, 1, true),
            "{label} sample {i}"
        );
        let mut got = Vec::new();
        reader.read_packet(0, i, &mut got).unwrap();
        assert_eq!(got, packet, "{label} packet {i}");
    }
    assert!(reader.read_packet(0, 3, &mut Vec::new()).is_err());
}

#[test]
fn owned_mp4_reader_handles_minimum_signed_trun_offset_without_overflow() {
    let min_underflow = bytes("mp4-signed-offset", "min-underflow.mp4");
    let caught = std::panic::catch_unwind(|| {
        Mp4Reader::open(Cursor::new(min_underflow), Limits::default())
            .err()
            .unwrap()
            .to_string()
    });
    if !ACCEPTANCE {
        assert!(
            caught.is_err(),
            "reproduction: unfixed owned_mp4::Mp4Reader panics on trun.data_offset == i32::MIN"
        );
        return;
    }
    let msg = caught.expect("owned_mp4::Mp4Reader must not panic on i32::MIN");
    assert!(msg.contains("before its base"), "{msg}");

    for c in manifest("mp4-signed-offset")["malformed"].as_array().unwrap() {
        let f = c["file"].as_str().unwrap();
        let err = Mp4Reader::open(Cursor::new(bytes("mp4-signed-offset", f)), Limits::default())
            .err()
            .unwrap();
        assert!(!err.is_unsupported(), "{f}: {err}");
        assert!(err.to_string().contains("before its base"), "{f}: {err}");
        let probe_err = probe_mp4(&fixture_path("mp4-signed-offset", f)).unwrap_err();
        assert!(probe_err.contains("before its base"), "{f}: {probe_err}");
    }

    let packet = bytes("mp4-signed-offset", "frame.vp9");
    for c in manifest("mp4-signed-offset")["controls"].as_array().unwrap() {
        let f = c.as_str().unwrap();
        let reader =
            Mp4Reader::open(Cursor::new(bytes("mp4-signed-offset", f)), Limits::default()).unwrap();
        verify_owned_packets(reader, f, &packet);
    }
    for key in ["valid-min", "valid-adjacent"] {
        let v = Sparse::virtual_file("mp4-signed-offset", key);
        let stats = v.stats.clone();
        let reader = Mp4Reader::open(BufReader::with_capacity(1024, v), Limits::default()).unwrap();
        verify_owned_packets(reader, key, &packet);
        assert!(stats.bytes.load(Ordering::Relaxed) <= 32 << 10);
    }
}

#[test]
fn owned_mp4_reader_enforces_fragment_description_trun_flags_and_box_extents() {
    if !ACCEPTANCE {
        // Reproduce silent acceptance of effective description 0, contradictory trun flags, and overlong boxes.
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes("mp4-fragment-description", "inherited-zero.mp4")),
                Limits::default()
            )
            .is_ok()
        );
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes(
                    "mp4-trun-flags",
                    "both-populated-first-sync-row-nonsync.mp4"
                )),
                Limits::default()
            )
            .is_ok()
        );
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes("mp4-tfhd-defaults", "tfhd-trailing-bytes.mp4")),
                Limits::default()
            )
            .is_ok()
        );
        return;
    }

    let packet = bytes("mp4-fragment-description", "frame.vp9");
    for c in manifest("mp4-fragment-description")["controls"]
        .as_array()
        .unwrap()
    {
        let f = c["file"].as_str().unwrap();
        let reader = Mp4Reader::open(
            Cursor::new(bytes("mp4-fragment-description", f)),
            Limits::default(),
        )
        .unwrap();
        verify_owned_packets(reader, f, &packet);
    }
    for c in manifest("mp4-fragment-description")["cases"]
        .as_array()
        .unwrap()
    {
        let f = c["file"].as_str().unwrap();
        let err = Mp4Reader::open(
            Cursor::new(bytes("mp4-fragment-description", f)),
            Limits::default(),
        )
        .err()
        .unwrap();
        assert!(err.is_unsupported(), "{f}: {err}");
        assert!(
            err.to_string()
                .contains("fragment sample description zero is not supported"),
            "{f}: {err}"
        );
    }
    let err = Mp4Reader::open(
        Cursor::new(bytes("mp4-fragment-description", "explicit-two.mp4")),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert!(!err.is_unsupported());
    assert!(err.to_string().contains("sample description index"));

    for c in manifest("mp4-trun-flags")["cases"].as_array().unwrap() {
        let f = c["file"].as_str().unwrap();
        let err = Mp4Reader::open(Cursor::new(bytes("mp4-trun-flags", f)), Limits::default())
            .err()
            .unwrap();
        assert!(!err.is_unsupported(), "{f}: {err}");
        assert!(
            err.to_string()
                .contains("states both first-sample flags and per-sample flags"),
            "{f}: {err}"
        );
    }

    for c in manifest("mp4-tfhd-defaults")["cases"].as_array().unwrap() {
        let f = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let err = Mp4Reader::open(
            Cursor::new(bytes("mp4-tfhd-defaults", f)),
            Limits::default(),
        )
        .err()
        .unwrap();
        assert!(!err.is_unsupported(), "{f}: {err}");
        assert!(err.to_string().contains(expected), "{f}: {err}");
    }
}

#[test]
fn owned_mp4_reader_enforces_per_traf_continuity_mfhd_and_unique_trex_ids() {
    if !ACCEPTANCE {
        // Reproduce cross-traf continuity failure on empty-leading-run-then-contiguous.mp4 and missing mfhd acceptance.
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes(
                    "mp4-multi-trun",
                    "empty-leading-run-then-contiguous.mp4"
                )),
                Limits::default()
            )
            .is_err()
        );
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes("mp4-mfhd-trex", "missing-mfhd.mp4")),
                Limits::default()
            )
            .is_ok()
        );
        return;
    }

    let packet = bytes("mp4-multi-trun", "frame.vp9");
    for dir in ["mp4-multi-trun", "mp4-mfhd-trex"] {
        let m = manifest(dir);
        for c in m["controls"].as_array().unwrap() {
            let f = c["file"].as_str().unwrap();
            let reader = Mp4Reader::open(Cursor::new(bytes(dir, f)), Limits::default()).unwrap();
            verify_owned_packets(reader, &format!("{dir}/{f}"), &packet);
        }
        for c in m["cases"].as_array().unwrap() {
            let f = c["file"].as_str().unwrap();
            let expected = c["expected_error"].as_str().unwrap();
            let err = Mp4Reader::open(Cursor::new(bytes(dir, f)), Limits::default())
                .err()
                .unwrap();
            assert!(!err.is_unsupported(), "{dir}/{f}: {err}");
            assert!(err.to_string().contains(expected), "{dir}/{f}: {err}");
        }
    }
}

#[test]
fn owned_mp4_reader_probe_and_decode_support_hybrid_fragments_and_refuse_corrupt_init_stbl() {
    if !ACCEPTANCE {
        let hybrid = bytes(
            "mp4-hybrid-fragments",
            "hybrid-one-in-stbl-two-in-fragments.mp4",
        );
        let reader = Mp4Reader::open(Cursor::new(hybrid), Limits::default()).unwrap();
        assert_eq!(reader.tracks()[0].samples.len(), 2);
        assert_eq!(reader.tracks()[0].duration, 1);
        assert!(
            Mp4Reader::open(
                Cursor::new(bytes(
                    "mp4-hybrid-fragments",
                    "corrupt-stco-outside-mdat-in-fragmented.mp4"
                )),
                Limits::default()
            )
            .is_ok()
        );
        return;
    }

    let packet = bytes("mp4-hybrid-fragments", "frame.vp9");
    let m = manifest("mp4-hybrid-fragments");
    for c in m["controls"].as_array().unwrap() {
        let f = c["file"].as_str().unwrap();
        let path = fixture_path("mp4-hybrid-fragments", f);
        let reader = Mp4Reader::open(
            Cursor::new(bytes("mp4-hybrid-fragments", f)),
            Limits {
                samples: 3,
                ..Default::default()
            },
        )
        .unwrap();
        verify_owned_packets(reader, f, &packet);

        let info = probe_mp4(&path).unwrap();
        assert_eq!(info.streams.len(), 1, "{f}");
        assert_eq!(info.streams[0].codec, "vp9", "{f}");
        assert_eq!(info.streams[0].duration, Some(3), "{f}");
        assert_eq!(info.duration_us, Some(60_000), "{f}");

        let stats = decode_video(&path).unwrap();
        assert_eq!(
            (stats.video_frames, stats.width, stats.height, stats.decode_errors),
            (3, 16, 16, 0),
            "{f}"
        );
    }

    for c in m["cases"].as_array().unwrap() {
        let f = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let path = fixture_path("mp4-hybrid-fragments", f);
        let err = Mp4Reader::open(
            Cursor::new(bytes("mp4-hybrid-fragments", f)),
            Limits::default(),
        )
        .err()
        .unwrap();
        assert!(!err.is_unsupported(), "{f}: {err}");
        assert!(err.to_string().contains(expected), "{f}: {err}");
        let probe_err = probe_mp4(&path).unwrap_err();
        assert!(probe_err.contains(expected), "{f}: {probe_err}");
        let decode_err = decode_video(&path).unwrap_err();
        assert!(decode_err.contains(expected), "{f}: {decode_err}");
    }
}
