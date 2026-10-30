use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
const VIDEO: &[u8] =
    include_bytes!("fixtures/playback-errors/mp4-interrupted-seek/three-colors.mp4");
const PACKETS: [&[u8]; 3] = [
    include_bytes!("fixtures/playback-errors/mp4-interrupted-seek/frame-0.packet"),
    include_bytes!("fixtures/playback-errors/mp4-interrupted-seek/frame-1.packet"),
    include_bytes!("fixtures/playback-errors/mp4-interrupted-seek/frame-2.packet"),
];
#[derive(Default)]
struct Control {
    remaining: AtomicUsize,
    fired: AtomicUsize,
}
struct Fault {
    source: Cursor<&'static [u8]>,
    control: Arc<Control>,
    moved: bool,
    mask: u8,
    kind: io::ErrorKind,
}
impl Fault {
    fn new(moved: bool, mask: u8, kind: io::ErrorKind) -> (Self, Arc<Control>) {
        let control = Arc::new(Control::default());
        (
            Self {
                source: Cursor::new(VIDEO),
                control: control.clone(),
                moved,
                mask,
                kind,
            },
            control,
        )
    }
}
impl Read for Fault {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.source.read(b)
    }
}
impl Seek for Fault {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let selected = match from {
            SeekFrom::Start(_) => self.mask & 1 != 0,
            SeekFrom::End(_) => self.mask & 2 != 0,
            _ => false,
        };
        if selected
            && self
                .control
                .remaining
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_ok()
        {
            if self.moved {
                self.source.seek(from)?;
            }
            self.control.fired.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::new(self.kind, "synthetic transient MP4 seek"));
        }
        self.source.seek(from)
    }
}
#[test]
fn opening_retries_interrupted_end_and_atom_seeks_in_both_parsers() {
    let mut failures = Vec::new();
    for moved in [false, true] {
        for mask in [1, 2] {
            for owned in [false, true] {
                let (source, c) = Fault::new(moved, mask, io::ErrorKind::Interrupted);
                c.remaining.store(2, Ordering::Relaxed);
                let result = if owned {
                    fvid_media::owned_mp4::Mp4Reader::open(source, Default::default())
                        .map(|r| r.tracks()[0].samples.len())
                        .map_err(|e| e.to_string())
                } else {
                    fvid::container::mp4::Mp4Reader::open(source, Default::default())
                        .map(|r| r.tracks()[0].samples.len())
                        .map_err(|e| e.to_string())
                };
                match result {
                    Ok(n) => {
                        assert_eq!(n, 3);
                        assert_eq!(c.fired.load(Ordering::Relaxed), 2)
                    }
                    Err(e) => {
                        failures.push(format!("moved={moved} mask={mask} owned={owned}: {e}"))
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "transient open failed: {failures:?}");
}
macro_rules! packets {
    ($reader:path) => {{
        for moved in [false, true] {
            let (source, c) = Fault::new(moved, 1, io::ErrorKind::Interrupted);
            let mut reader = <$reader>::open(source, Default::default()).unwrap();
            let mut bytes = Vec::new();
            reader.read_packet(0, 0, &mut bytes).unwrap();
            assert_eq!(bytes, PACKETS[0]);
            c.remaining.store(2, Ordering::Relaxed);
            reader
                .read_packet(0, 2, &mut bytes)
                .expect("transient full-packet seek should recover");
            assert_eq!(bytes, PACKETS[2]);
            assert_eq!(c.fired.load(Ordering::Relaxed), 2);
            c.remaining.store(2, Ordering::Relaxed);
            let mut part = [0xa5; 7];
            assert_eq!(
                reader
                    .read_packet_range(0, 0, 1, &mut part)
                    .expect("transient range seek should recover"),
                7
            );
            assert_eq!(part, &PACKETS[0][1..8]);
            assert_eq!(c.fired.load(Ordering::Relaxed), 4);
            reader.read_packet(0, 1, &mut bytes).unwrap();
            assert_eq!(bytes, PACKETS[1]);
            reader.read_packet(0, 2, &mut bytes).unwrap();
            assert_eq!(bytes, PACKETS[2]);
        }
    }};
}
#[test]
fn root_packet_and_range_seeks_retry_interruption() {
    packets!(fvid::container::mp4::Mp4Reader<Fault>);
}
#[test]
fn owned_packet_and_range_seeks_retry_interruption() {
    packets!(fvid_media::owned_mp4::Mp4Reader<Fault>);
}
#[test]
fn permanent_seek_failures_still_propagate_and_allow_recovery() {
    for kind in [io::ErrorKind::Other, io::ErrorKind::PermissionDenied] {
        let (source, c) = Fault::new(true, 1, kind);
        let mut reader = fvid::container::mp4::Mp4Reader::open(source, Default::default()).unwrap();
        let mut bytes = Vec::new();
        reader.read_packet(0, 0, &mut bytes).unwrap();
        c.remaining.store(1, Ordering::Relaxed);
        assert!(reader.read_packet(0, 2, &mut bytes).is_err());
        assert!(bytes.is_empty());
        assert_eq!(c.fired.load(Ordering::Relaxed), 1);
        reader.read_packet(0, 1, &mut bytes).unwrap();
        assert_eq!(bytes, PACKETS[1]);
    }
}
#[test]
fn software_playback_keeps_pixels_timestamps_seek_rewind_and_eof_after_interruption() {
    use fvid::playback_mp4::Mp4VideoReader;
    let (source, c) = Fault::new(true, 1, io::ErrorKind::Interrupted);
    let mut reader = Mp4VideoReader::open_software(source, Default::default(), 32 << 20).unwrap();
    for pass in 0..2 {
        if pass != 0 {
            reader.rewind();
        }
        for i in [0, 2, 1, 2] {
            c.remaining.store(2, Ordering::Relaxed);
            // Cover random seeks and the contiguous final sample without forcing I/O.
            reader.seek_to_sync(i);
            let frame = reader
                .read_frame()
                .expect("interrupted playback seek should recover")
                .unwrap();
            assert_eq!(frame.presentation_time.ticks, i as i64);
            assert_eq!(frame.presentation_time.timescale, 50);
            assert_eq!(frame.duration.ticks, 1);
            let bytes: Vec<_> = frame
                .picture
                .y
                .iter()
                .chain(&frame.picture.cb)
                .chain(&frame.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            let oracle = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                format!("tests/fixtures/playback-errors/mp4-interrupted-seek/frame-{i}.yuv16"),
            ))
            .unwrap();
            assert_eq!(bytes, oracle);
        }
        assert!(reader.read_frame().unwrap().is_none());
        assert!(reader.read_frame().unwrap().is_none());
    }
    assert!(c.fired.load(Ordering::Relaxed) >= 4);
}
