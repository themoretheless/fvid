use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder},
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-inband-base-bootstrap/manifest.json")).unwrap(),
    )
    .unwrap();
    let c = v.as_array().unwrap().clone();
    assert_eq!(c.len(), 48);
    c
}
fn raw_picture(p: &fvid::codec::avc_picture::IntraPicture) -> Vec<u8> {
    let wide = p.bit_depth.max(p.chroma_bit_depth) > 8;
    let mut out = Vec::new();
    for plane in [&p.y, &p.cb, &p.cr] {
        for &v in plane {
            if wide {
                out.extend_from_slice(&v.to_le_bytes());
            } else {
                out.push(v as u8);
            }
        }
    }
    out
}
#[test]
fn empty_and_partial_hev1_configuration_bootstraps_native_pixels_and_seek() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let first = c["first_sample"].as_i64().unwrap();
        let count = c["frames"].as_u64().unwrap() as usize;
        let bytes = std::fs::read(
            root()
                .join("hevc-inband-base-bootstrap")
                .join(format!("{name}.mp4")),
        )
        .unwrap();
        let demux = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        assert_eq!(&demux.tracks()[0].codec, b"hev1");
        let cfg = HevcConfig::parse(&demux.tracks()[0].configuration).unwrap();
        assert_eq!(
            cfg.arrays
                .iter()
                .map(|a| a.nal_type as u64)
                .collect::<Vec<_>>(),
            c["configuration_types"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>()
        );
        let expected =
            std::fs::read(root().join(format!("{}.yuv", c["seed"].as_str().unwrap()))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET)
                .unwrap_or_else(|e| panic!("open {name}: {e}"));
        for round in 0..3 {
            if round == 1 {
                reader.rewind();
            }
            if round == 2 {
                assert_eq!(reader.seek_to_sync(first), first, "{name}");
            }
            reader.active_colour().unwrap();
            let _ = reader.bitstream_colour();
            let _ = reader.bitstream_hdr();
            let mut actual = Vec::new();
            for i in 0..count {
                let f = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("read {name}/{i}: {e}"))
                    .unwrap();
                assert_eq!(f.sample_index, first as usize + i);
                assert_eq!(f.presentation_time.ticks, first + i as i64);
                assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(reader.selected_layer_id(), None);
                actual.extend(raw_picture(&f.picture));
            }
            assert!(reader.read_frame().unwrap().is_none());
            assert_eq!(actual, expected, "{name}");
        }
    }
}
#[test]
fn strict_configuration_controls_validate_every_original_coded_seed() {
    let mut seeds = std::collections::BTreeSet::new();
    for c in cases() {
        seeds.insert(c["seed"].as_str().unwrap().to_string());
    }
    assert_eq!(seeds.len(), 6);
    for seed in seeds {
        let bytes = std::fs::read(root().join(format!("{seed}.mp4"))).unwrap();
        let mut demux = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&demux.tracks()[0].configuration, BUDGET).unwrap();
        let mut actual = Vec::new();
        for i in 0..demux.tracks()[0].samples.len() {
            let mut packet = Vec::new();
            demux.read_packet(0, i, &mut packet).unwrap();
            let f = decoder.decode_packet(&packet).unwrap().unwrap();
            let wide = f.picture.depth.into_iter().max().unwrap() > 8;
            for plane in &f.picture.planes {
                for &v in plane.samples() {
                    if wide {
                        actual.extend_from_slice(&v.to_le_bytes());
                    } else {
                        actual.push(v as u8);
                    }
                }
            }
        }
        assert_eq!(
            actual,
            std::fs::read(root().join(format!("{seed}.yuv"))).unwrap(),
            "{seed}"
        );
    }
}

#[test]
fn native_inband_api_exposes_readiness_and_restores_initial_partial_banks() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let bytes = std::fs::read(
            root()
                .join("hevc-inband-base-bootstrap")
                .join(format!("{name}.mp4")),
        )
        .unwrap();
        let mut demux = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
        let cfg = demux.tracks()[0].configuration.clone();
        let ready = c["configuration_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == 33)
            && c["configuration_types"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == 34);
        assert_eq!(
            HevcDecoder::from_configuration(&cfg, BUDGET).is_ok(),
            ready,
            "strict {name}"
        );
        let mut decoder = HevcDecoder::from_inband_configuration(&cfg, BUDGET).unwrap();
        let expected =
            std::fs::read(root().join(format!("{}.yuv", c["seed"].as_str().unwrap()))).unwrap();
        for _ in 0..2 {
            assert_eq!(decoder.try_parameters().is_some(), ready, "initial {name}");
            let mut actual = Vec::new();
            let mut frames = 0;
            for i in 0..demux.tracks()[0].samples.len() {
                let mut packet = Vec::new();
                demux.read_packet(0, i, &mut packet).unwrap();
                if let Some(f) = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("native {name}/{i}: {e}"))
                {
                    frames += 1;
                    assert!(decoder.try_parameters().is_some());
                    let wide = f.picture.depth.into_iter().max().unwrap() > 8;
                    for p in &f.picture.planes {
                        for &v in p.samples() {
                            if wide {
                                actual.extend_from_slice(&v.to_le_bytes());
                            } else {
                                actual.push(v as u8);
                            }
                        }
                    }
                } else {
                    assert!(i < c["first_sample"].as_u64().unwrap() as usize);
                }
            }
            assert_eq!(frames, c["frames"].as_u64().unwrap());
            assert_eq!(actual, expected, "native {name}");
            decoder.reset();
        }
    }
}

#[test]
fn unresolved_pps_reception_uses_aggregate_budget_and_error_requires_reset() {
    let path = root().join("hevc-inband-base-bootstrap");
    let mut input = Mp4Reader::open(
        Cursor::new(std::fs::read(path.join("unresolved-pps-budget.mp4")).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let mut decoder =
        HevcDecoder::from_inband_configuration(&input.tracks()[0].configuration, 128 << 10)
            .unwrap();
    let mut accepted = 0;
    let mut refused = false;
    for i in 0..32 {
        let mut packet = Vec::new();
        input.read_packet(0, i, &mut packet).unwrap();
        assert!(packet.len() < 8192);
        match decoder.decode_packet(&packet) {
            Ok(None) => accepted += 1,
            Err(e) => {
                assert!(e.to_string().contains("budget"), "{e}");
                refused = true;
                break;
            }
            Ok(Some(_)) => panic!("parameter-only packet cannot publish pixels"),
        }
    }
    assert!(
        refused && accepted > 1 && accepted < 32,
        "must refuse accumulated PPS bytes, not one packet"
    );
    let recovery = std::fs::read(path.join("recovery.packet")).unwrap();
    assert!(
        decoder
            .decode_packet(&recovery)
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    assert!(decoder.try_parameters().is_none());
    assert!(decoder.decode_packet(&recovery).unwrap().is_none());
    assert!(decoder.try_parameters().is_some());
}
