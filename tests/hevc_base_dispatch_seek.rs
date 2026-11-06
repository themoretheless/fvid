use fvid::{playback::video::webm::WebmVideoReader, playback_native::RawFrame};
use std::{io::Cursor, path::PathBuf};
const B: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-base-dispatch-seek")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    let v = m["cases"].as_array().unwrap().clone();
    assert_eq!(v.len(), 10);
    v
}
fn open(c: &serde_json::Value) -> fvid::Result<WebmVideoReader<Cursor<Vec<u8>>>> {
    WebmVideoReader::open_software(Cursor::new(bytes(c["file"].as_str().unwrap())), B)
}
fn picture(r: &mut WebmVideoReader<Cursor<Vec<u8>>>, c: &serde_json::Value, i: usize) {
    let f = &c["frames"][i];
    let Some(RawFrame::Planar(p)) = r.read_frame_raw().unwrap() else {
        panic!("source depth")
    };
    assert_eq!(
        p.frame.data,
        bytes(f["oracle"].as_str().unwrap()),
        "{} frame{i}",
        c["file"]
    );
    assert_eq!(p.depth, f["depth"].as_u64().unwrap() as u8);
    assert_eq!(
        [p.frame.width as u64, p.frame.height as u64],
        [
            c["size"][0].as_u64().unwrap(),
            c["size"][1].as_u64().unwrap()
        ]
    );
    assert_eq!(
        r.frame_interval(),
        Some((
            u128::from(f["start_ns"].as_u64().unwrap()),
            u128::from(f["end_ns"].as_u64().unwrap()),
            1_000_000_000
        ))
    );
}
#[test]
fn healthy_controls_keep_every_independent_picture_and_clock() {
    for c in cases().iter().filter(|c| c["mode"] == "healthy") {
        let mut r = open(c).unwrap();
        for _ in 0..2 {
            for i in 0..c["frames"].as_array().unwrap().len() {
                picture(&mut r, c, i)
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            r.rewind();
        }
    }
}
#[test]
fn damaged_pictures_keep_the_specific_nal_header_rbsp_or_lookup_failure() {
    for c in cases().iter().filter(|c| {
        matches!(
            c["mode"].as_str().unwrap(),
            "header-only" | "bad-escape" | "absent-pps"
        )
    }) {
        let hex = c["first_nal_hex"].as_str().unwrap();
        let nal: Vec<_> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        let header = fvid::codec::hevc_nal::NalHeader::parse(&nal).unwrap();
        assert_eq!(header.layer_id, 0);
        assert!(header.is_vcl());
        if c["mode"] == "absent-pps" {
            assert_eq!(
                fvid::codec::hevc_slice::SliceHeader::parameter_set_id(&nal).unwrap(),
                63
            );
        } else {
            assert!(fvid::codec::hevc_nal::NalRbsp::parse(&nal, B).is_err());
        }
    }
}
#[test]
fn corrupted_old_pictures_refuse_sequentially_but_allow_exact_new_cvs_seek() {
    let mut failures = Vec::new();
    for c in cases().iter().filter(|c| {
        matches!(
            c["mode"].as_str().unwrap(),
            "header-only" | "bad-escape" | "absent-pps"
        )
    }) {
        let mut r = match open(c) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{}:open:{e}", c["file"]));
                continue;
            }
        };
        for used in [false, true] {
            r.rewind();
            if used {
                assert!(
                    r.read_frame_raw().is_err(),
                    "old picture must refuse: {}",
                    c["file"]
                );
            }
            let target = c["target_ns"].as_i64().unwrap();
            assert_eq!(r.seek_to_sync(target).unwrap(), target);
            for i in
                c["new_start"].as_u64().unwrap() as usize..c["frames"].as_array().unwrap().len()
            {
                picture(&mut r, c, i)
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            assert!(r.read_frame_raw().unwrap().is_none());
        }
        r.rewind();
        assert!(r.read_frame_raw().is_err());
    }
    assert!(
        failures.is_empty(),
        "base-only dispatch rejects old picture bodies: {failures:?}"
    );
}
#[test]
fn corrupt_received_parameters_and_forbidden_nal_headers_still_refuse() {
    for c in cases() {
        if c["mode"] == "bad-header" {
            assert!(open(&c).is_err());
        }
        if c["mode"] == "bad-parameters" {
            let mut r = open(&c).unwrap();
            assert!(r.seek_to_sync(c["target_ns"].as_i64().unwrap()).is_err());
            assert!(
                r.read_frame_raw()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            r.rewind();
            picture(&mut r, &c, 0);
        }
    }
}

#[test]
fn base_only_dispatch_keeps_bounded_source_io() {
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };
    struct Counted {
        cursor: Cursor<Vec<u8>>,
        bytes: Arc<AtomicU64>,
    }
    impl Read for Counted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.cursor.read(out)?;
            self.bytes.fetch_add(n as u64, Ordering::Relaxed);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(to)
        }
    }
    for c in cases().iter().filter(|c| c["mode"] == "healthy") {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let length = data.len();
        let count = Arc::new(AtomicU64::new(0));
        let mut r = WebmVideoReader::open_software(
            Counted {
                cursor: Cursor::new(data),
                bytes: count.clone(),
            },
            B,
        )
        .unwrap();
        let received = count.load(Ordering::Relaxed);
        println!("dispatch-open {name} {length} {received}");
        assert!(
            received <= 3 * length as u64,
            "{name}: excess constructor IO {received}/{length}"
        );
        assert!(r.read_frame_raw().unwrap().is_some());
    }
}

#[test]
fn default_backend_keeps_the_complete_parameter_changing_sequence() {
    use fvid::playback_native::NativeReader;
    for c in cases().iter().filter(|c| c["mode"] == "healthy") {
        let name = c["file"].as_str().unwrap();
        let mut reference = NativeReader::software(Cursor::new(bytes(name)), B).unwrap();
        let mut expected = Vec::new();
        while let Some(frame) = reference.read_frame_raw().unwrap() {
            expected.push((
                reference.frame_interval().unwrap(),
                frame.into_rgb(B).unwrap(),
            ));
        }
        let mut r = NativeReader::new(Cursor::new(bytes(name)), B).unwrap();
        let hardware = r.hardware_accelerated();
        let mut received = Vec::new();
        while let Some(frame) = r.read_frame_raw().unwrap() {
            received.push((r.frame_interval().unwrap(), frame.into_rgb(B).unwrap()));
        }
        assert_eq!(
            received, expected,
            "{name}: default backend hardware={hardware} lost pictures or changed exact pixels/intervals"
        );
    }
}
