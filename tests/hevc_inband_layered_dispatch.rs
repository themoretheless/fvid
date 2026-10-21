use fvid::{
    codec::{config::HevcConfig, hevc_vps::Vps},
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::{LayeredMp4VideoReader, Mp4VideoReader},
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> Vec<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-inband-layered-dispatch/manifest.json")).unwrap(),
    )
    .unwrap();
    let rows = value.as_array().unwrap().clone();
    assert_eq!(rows.len(), 25);
    rows
}
fn movie(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        root()
            .join("hevc-inband-layered-dispatch")
            .join(format!("{}.mp4", c["name"].as_str().unwrap())),
    )
    .unwrap()
}
fn oracle(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap()
}
#[test]
fn automatic_dispatch_uses_first_active_vps_and_inband_layered_parameters() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let bytes = movie(&c);
        let demux = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        assert_eq!(&demux.tracks()[0].codec, b"hev1");
        let cfg = HevcConfig::parse(&demux.tracks()[0].configuration).unwrap();
        let vps: Vec<_> = cfg
            .arrays
            .iter()
            .filter(|a| a.nal_type == 32)
            .flat_map(|a| a.units.iter())
            .collect();
        if c["variant"].as_str().unwrap().starts_with("empty") {
            assert!(vps.is_empty());
        } else {
            assert_eq!(
                Vps::parse_layered_metadata(vps[0], BUDGET)
                    .unwrap()
                    .layers
                    .effective_max_layers(),
                1
            );
        }
        let golden = oracle(&c);
        let count = c["frames"].as_u64().unwrap() as usize;
        let first = c["first_sample"].as_i64().unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET)
                .unwrap_or_else(|e| panic!("open {name}: {e}"));
        for round in 0..3 {
            if round == 1 {
                reader.rewind();
            }
            if round == 2 {
                assert_eq!(reader.seek_to_sync(first), first, "seek {name}");
            }
            for i in 0..count {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("read {name}/{i}: {e}"))
                    .unwrap();
                assert_eq!(reader.selected_layer_id(), Some(3), "{name}");
                assert_eq!(frame.sample_index, i + first as usize);
                assert_eq!(frame.presentation_time.ticks, first + i as i64);
                assert_eq!(frame.presentation_time.timescale, 25);
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                let packed = frame.packed.unwrap();
                let size = packed.frame.data.len();
                assert_eq!(
                    packed.frame.data,
                    &golden[i * size..(i + 1) * size],
                    "pixels {name}/{i}"
                );
            }
            assert!(reader.read_frame().unwrap().is_none(), "EOF {name}");
        }
    }
}
#[test]
fn explicit_layered_reader_controls_validate_coded_streams_without_configuration_bootstrap() {
    for c in cases()
        .into_iter()
        .filter(|c| !c["variant"].as_str().unwrap().starts_with("empty"))
    {
        let name = c["name"].as_str().unwrap();
        let golden = oracle(&c);
        let first = c["first_sample"].as_u64().unwrap() as usize;
        let mut reader =
            LayeredMp4VideoReader::open(Cursor::new(movie(&c)), Limits::default(), 1, 0, BUDGET)
                .unwrap_or_else(|e| panic!("explicit {name}: {e}"));
        let mut seen = 0;
        while let Some(frame) = reader
            .read_frame()
            .unwrap_or_else(|e| panic!("explicit read {name}: {e}"))
        {
            if frame.decoded.layer_id == 3 {
                seen += 1;
            }
            let i = frame.sample_index - first;
            let packed = frame.into_video_frame().unwrap().packed.unwrap();
            let size = packed.frame.data.len();
            assert_eq!(
                packed.frame.data,
                &golden[i * size..(i + 1) * size],
                "explicit pixels {name}"
            );
        }
        assert_eq!(seen, c["frames"].as_u64().unwrap() as usize);
    }
}

#[test]
fn complete_hvc1_keeps_the_existing_metadata_only_open_path() {
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    struct Observed {
        cursor: Cursor<Vec<u8>>,
        payload: std::ops::Range<u64>,
        bytes: Arc<AtomicUsize>,
    }
    impl Read for Observed {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let start = self.cursor.position();
            let count = self.cursor.read(out)?;
            let overlap = (start + count as u64)
                .min(self.payload.end)
                .saturating_sub(start.max(self.payload.start));
            self.bytes.fetch_add(overlap as usize, Ordering::Relaxed);
            Ok(count)
        }
    }
    impl Seek for Observed {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(pos)
        }
    }
    let data = std::fs::read(root().join("hevc-pcm-reference-rext8.mp4")).unwrap();
    let demux = Mp4Reader::open(Cursor::new(&data), Limits::default()).unwrap();
    assert_eq!(&demux.tracks()[0].codec, b"hvc1");
    let samples = &demux.tracks()[0].samples;
    let first = samples.get(0).unwrap();
    let last = samples.get(samples.len() - 1).unwrap();
    let bytes = Arc::new(AtomicUsize::new(0));
    let observed = Observed {
        cursor: Cursor::new(data.clone()),
        payload: first.offset..last.offset + u64::from(last.size),
        bytes: bytes.clone(),
    };
    let mut reader = Mp4VideoReader::open_software(observed, Limits::default(), BUDGET).unwrap();
    assert_eq!(
        bytes.load(Ordering::Relaxed),
        0,
        "complete hvc1 must not scan coded packets on open"
    );
    assert!(reader.read_frame().unwrap().is_some());
    assert!(
        bytes.load(Ordering::Relaxed) > 0,
        "probe must observe actual picture reads"
    );
    assert_eq!(reader.selected_layer_id(), None);
}
