use fvid::{
    codec::hevc_decoder::HevcDecoder,
    color::hdr::HdrMetadata,
    container::mp4::{Limits, Mp4Reader},
    playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader},
    playback_native::RawFrame,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn data(name: &str) -> Vec<u8> {
    std::fs::read(root().join("hevc-hdr-seek").join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    let m: serde_json::Value = serde_json::from_slice(&data("manifest.json")).unwrap();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 9);
    assert_eq!(cases.iter().filter(|c| c["kind"] == "mp4").count(), 6);
    assert_eq!(cases.iter().filter(|c| c["kind"] == "mkv").count(), 3);
    assert_eq!(m["hdr"].as_array().unwrap().len(), 3);
    let pictures = m["frames"].as_u64().unwrap() as usize;
    assert!(pictures > 0);
    for case in cases {
        let metadata = match case["layout"].as_str().unwrap() {
            "inline" => 0,
            "prefix" => 1,
            "split" => 2,
            _ => panic!("unknown layout"),
        };
        for cycle in 0..3 {
            let expected: Vec<_> = (0..pictures)
                .map(|i| (cycle * (pictures + metadata) + metadata + i) as u64)
                .collect();
            let actual: Vec<_> = case["indices"][cycle]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect();
            assert_eq!(
                actual, expected,
                "fixture must retain metadata-only packet positions"
            );
        }
    }
    m
}
fn hdr(actual: HdrMetadata, expected: &serde_json::Value) {
    let master = actual
        .mastering
        .expect("received mastering volume must survive seek");
    for (point, value) in [
        (master.green, &expected["primaries"][0]),
        (master.blue, &expected["primaries"][1]),
        (master.red, &expected["primaries"][2]),
        (master.white, &expected["white"]),
    ] {
        assert!((point.x - value[0].as_u64().unwrap() as f64 / 50000.0).abs() < 1e-9);
        assert!((point.y - value[1].as_u64().unwrap() as f64 / 50000.0).abs() < 1e-9);
    }
    assert!(
        (master.max_luminance as f64 - expected["maximum"].as_u64().unwrap() as f64 / 10000.0)
            .abs()
            < 0.001
    );
    assert!(
        (master.min_luminance as f64 - expected["minimum"].as_u64().unwrap() as f64 / 10000.0)
            .abs()
            < 1e-8
    );
    assert_eq!(
        actual.light.max_cll,
        expected["cll"].as_u64().unwrap() as f32
    );
    assert_eq!(
        actual.light.max_fall,
        expected["fall"].as_u64().unwrap() as f32
    );
}
fn oracle(m: &serde_json::Value) -> Vec<u8> {
    std::fs::read(root().join(format!("{}.yuv", m["seed"].as_str().unwrap()))).unwrap()
}
#[test]
fn decoder_priming_restores_split_hdr_and_is_atomic_on_parameter_error() {
    let m = manifest();
    let source = Mp4Reader::open(
        Cursor::new(
            std::fs::read(root().join(format!("{}.mp4", m["seed"].as_str().unwrap()))).unwrap(),
        ),
        Limits::default(),
    )
    .unwrap();
    let mut d = HevcDecoder::from_configuration(&source.tracks()[0].configuration, BUDGET).unwrap();
    assert_eq!(d.hdr(), HdrMetadata::default());
    d.prime_parameters(&data("0-master.packet")).unwrap();
    assert!(d.hdr().mastering.is_some());
    assert_eq!(d.hdr().light.max_cll, 0.0);
    d.prime_parameters(&data("0-light.packet")).unwrap();
    hdr(d.hdr(), &m["hdr"][0]);
    assert!(d.prime_parameters(&data("bad-parameters.packet")).is_err());
    hdr(d.hdr(), &m["hdr"][0]);
    assert!(
        d.prime_parameters(&data("1-full.packet"))
            .unwrap_err()
            .to_string()
            .contains("requires reset")
    );
    d.reset();
    assert_eq!(d.hdr(), HdrMetadata::default());
    d.prime_parameters(&data("2-full.packet")).unwrap();
    hdr(d.hdr(), &m["hdr"][2]);
    let mut configured = HevcDecoder::from_configuration(&data("configured.hvcc"), BUDGET).unwrap();
    hdr(configured.hdr(), &m["hdr"][0]);
    configured.prime_parameters(&data("1-full.packet")).unwrap();
    hdr(configured.hdr(), &m["hdr"][1]);
    configured.reset();
    hdr(configured.hdr(), &m["hdr"][0]);
}
#[test]
fn mp4_sequential_controls_and_seek_match_pixels_and_latest_hdr() {
    let m = manifest();
    let expected = oracle(&m);
    let count = m["frames"].as_u64().unwrap() as usize;
    let stride = expected.len() / count;
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "mp4")
    {
        let name = case["file"].as_str().unwrap();
        let mut r =
            Mp4VideoReader::open_software(Cursor::new(data(name)), Limits::default(), BUDGET)
                .unwrap();
        for _ in 0..2 {
            for cycle in 0..3 {
                for i in 0..count {
                    let frame = r.read_frame().unwrap().unwrap();
                    let mut raw = Vec::new();
                    frame.picture.write_planar(&mut raw).unwrap();
                    assert_eq!(
                        raw,
                        expected[i * stride..(i + 1) * stride],
                        "{name}/{cycle}/{i}"
                    );
                    assert_eq!(
                        frame.sample_index,
                        case["indices"][cycle][i].as_u64().unwrap() as usize
                    );
                    {
                        assert!(
                            r.bitstream_hdr().mastering.is_some(),
                            "{name}: sequential/rewind cycle {cycle}"
                        );
                        assert_eq!(
                            r.bitstream_hdr().light.max_cll,
                            m["hdr"][cycle]["cll"].as_u64().unwrap() as f32,
                            "{name}: cycle {cycle}, picture {i} HDR must belong to displayed frame"
                        );
                        hdr(r.bitstream_hdr(), &m["hdr"][cycle]);
                    }
                }
            }
            assert!(r.read_frame().unwrap().is_none());
            hdr(r.bitstream_hdr(), &m["hdr"][2]);
            assert!(r.read_frame().unwrap().is_none());
            hdr(r.bitstream_hdr(), &m["hdr"][2]);
            r.rewind();
        }
        for cycle in [2usize, 0, 1, 2] {
            let target = case["indices"][cycle][0].as_i64().unwrap();
            assert_eq!(r.seek_to_sync(target), target);
            for i in 0..count {
                let frame = r.read_frame().unwrap().unwrap();
                let mut raw = Vec::new();
                frame.picture.write_planar(&mut raw).unwrap();
                assert_eq!(raw, expected[i * stride..(i + 1) * stride]);
                {
                    assert!(
                        r.bitstream_hdr().mastering.is_some(),
                        "{name}: seek cycle {cycle}"
                    );
                    assert_eq!(
                        r.bitstream_hdr().light.max_cll,
                        m["hdr"][cycle]["cll"].as_u64().unwrap() as f32,
                        "{name}: cycle {cycle}, picture {i} HDR must belong to displayed frame"
                    );
                    hdr(r.bitstream_hdr(), &m["hdr"][cycle]);
                }
            }
        }
    }
}
#[test]
fn matroska_sequential_controls_and_seek_match_pixels_and_latest_hdr() {
    let m = manifest();
    let expected = oracle(&m);
    let count = m["frames"].as_u64().unwrap() as usize;
    let stride = expected.len() / count;
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "mkv")
    {
        let name = case["file"].as_str().unwrap();
        let mut r = WebmVideoReader::open_software(Cursor::new(data(name)), BUDGET).unwrap();
        for _ in 0..2 {
            for cycle in 0..3 {
                for i in 0..count {
                    let RawFrame::Planar(p) = r.read_frame_raw().unwrap().unwrap() else {
                        panic!("native packed HEVC")
                    };
                    assert_eq!(
                        p.frame.data,
                        expected[i * stride..(i + 1) * stride],
                        "{name}/{cycle}/{i}"
                    );
                    {
                        assert!(
                            r.bitstream_hdr().mastering.is_some(),
                            "{name}: sequential/rewind cycle {cycle}"
                        );
                        assert_eq!(
                            r.bitstream_hdr().light.max_cll,
                            m["hdr"][cycle]["cll"].as_u64().unwrap() as f32,
                            "{name}: cycle {cycle}, picture {i} HDR must belong to displayed frame"
                        );
                        hdr(r.bitstream_hdr(), &m["hdr"][cycle]);
                    }
                }
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            hdr(r.bitstream_hdr(), &m["hdr"][2]);
            assert!(r.read_frame_raw().unwrap().is_none());
            hdr(r.bitstream_hdr(), &m["hdr"][2]);
            r.rewind();
        }
        for cycle in [2usize, 0, 1, 2] {
            let target = case["indices"][cycle][0].as_i64().unwrap() * 40_000_000;
            assert_eq!(r.seek_to_sync(target).unwrap(), target);
            for i in 0..count {
                let RawFrame::Planar(p) = r.read_frame_raw().unwrap().unwrap() else {
                    panic!("native packed HEVC")
                };
                assert_eq!(p.frame.data, expected[i * stride..(i + 1) * stride]);
                {
                    assert!(
                        r.bitstream_hdr().mastering.is_some(),
                        "{name}: seek cycle {cycle}"
                    );
                    assert_eq!(
                        r.bitstream_hdr().light.max_cll,
                        m["hdr"][cycle]["cll"].as_u64().unwrap() as f32,
                        "{name}: cycle {cycle}, picture {i} HDR must belong to displayed frame"
                    );
                    hdr(r.bitstream_hdr(), &m["hdr"][cycle]);
                }
            }
        }
    }
}
