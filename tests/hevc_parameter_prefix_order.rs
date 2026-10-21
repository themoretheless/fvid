use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_pps::Pps},
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn pixels(p: &fvid::codec::avc_picture::IntraPicture) -> Vec<u8> {
    let mut out = Vec::new();
    for plane in [&p.y, &p.cb, &p.cr] {
        for &v in plane {
            if p.bit_depth.max(p.chroma_bit_depth) > 8 {
                out.extend_from_slice(&v.to_le_bytes());
            } else {
                out.push(v as u8);
            }
        }
    }
    out
}
#[test]
fn updated_pps_is_bound_to_received_sps_before_picture_activation() {
    let old_bytes = std::fs::read(root().join("hevc-pcm-reference-rext8.mp4")).unwrap();
    let new_bytes = std::fs::read(root().join("hevc-scc-act-slice-y10-c10.mp4")).unwrap();
    let old = Mp4Reader::open(Cursor::new(old_bytes), Limits::default()).unwrap();
    let new = Mp4Reader::open(Cursor::new(new_bytes), Limits::default()).unwrap();
    let old_decoder =
        HevcDecoder::from_configuration(&old.tracks()[0].configuration, BUDGET).unwrap();
    let new_decoder =
        HevcDecoder::from_configuration(&new.tracks()[0].configuration, BUDGET).unwrap();
    let cfg = HevcConfig::parse(&new.tracks()[0].configuration).unwrap();
    let pps = cfg.arrays.iter().find(|a| a.nal_type == 34).unwrap().units[0];
    // Establish the specific stale-context failure rather than an entropy error.
    assert_eq!(old_decoder.parameters().0.id, new_decoder.parameters().0.id);
    assert!(Pps::parse(pps, old_decoder.parameters().0, BUDGET).is_err());
    assert!(Pps::parse(pps, new_decoder.parameters().0, BUDGET).is_ok());
    let cases: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(root().join("hevc-parameter-prefix-order/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(cases.len(), 6);
    let old_pixels = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    let new_pixels = std::fs::read(root().join("hevc-scc-act-slice-y10-c10.yuv")).unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let data = std::fs::read(
            root()
                .join("hevc-parameter-prefix-order")
                .join(format!("{name}.mp4")),
        )
        .unwrap();
        let old_count = case["old_frames"].as_u64().unwrap() as usize;
        let new_count = case["new_frames"].as_u64().unwrap() as usize;
        let prefix = usize::from(case["separate"].as_bool().unwrap());
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(data), Limits::default(), BUDGET).unwrap();
        for pass in 0..3 {
            if pass == 1 {
                reader.rewind();
            }
            if pass == 2 {
                assert_eq!(
                    reader.seek_to_sync((old_count + prefix) as i64),
                    (old_count + prefix) as i64
                );
            }
            let start = if pass == 2 { old_count } else { 0 };
            let mut before = Vec::new();
            let mut after = Vec::new();
            for i in start..old_count + new_count {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name}/{pass}/{i}: {e}"))
                    .unwrap();
                let index = i + if i >= old_count { prefix } else { 0 };
                assert_eq!(frame.sample_index, index, "{name}");
                assert_eq!(frame.presentation_time.ticks, index as i64, "{name}");
                assert_eq!(
                    frame.duration.nanoseconds().unwrap(),
                    if prefix == 1 && i + 1 == old_count {
                        80_000_000
                    } else {
                        40_000_000
                    }
                );
                if i < old_count {
                    before.extend(pixels(&frame.picture));
                } else {
                    after.extend(pixels(&frame.picture));
                }
            }
            if pass < 2 {
                assert_eq!(before, old_pixels, "{name}");
            }
            assert_eq!(after, new_pixels, "{name}");
            assert!(reader.read_frame().unwrap().is_none());
        }
    }
}

#[test]
fn both_native_apis_stage_reordered_parameters_without_mutating_header_queries() {
    let cases: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(root().join("hevc-parameter-prefix-order/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut expected = std::fs::read(root().join("hevc-pcm-reference-rext8.yuv")).unwrap();
    expected.extend(std::fs::read(root().join("hevc-scc-act-slice-y10-c10.yuv")).unwrap());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let data = std::fs::read(
            root()
                .join("hevc-parameter-prefix-order")
                .join(format!("{name}.mp4")),
        )
        .unwrap();
        let mut source = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        for inband in [false, true] {
            let config = &source.tracks()[0].configuration;
            let mut decoder = if inband {
                HevcDecoder::from_inband_configuration(config, BUDGET)
            } else {
                HevcDecoder::from_configuration(config, BUDGET)
            }
            .unwrap();
            let initial = decoder.parameters().0.clone();
            for _ in 0..2 {
                let mut actual = Vec::new();
                let mut pictures = 0;
                let mut packet = Vec::new();
                for i in 0..source.tracks()[0].samples.len() {
                    source.read_packet(0, i, &mut packet).unwrap();
                    let before = (
                        decoder.parameters().0.clone(),
                        decoder.parameters().1.clone(),
                    );
                    let headers = decoder
                        .slice_headers(&packet)
                        .unwrap_or_else(|e| panic!("{name}/{i}: {e}"));
                    assert_eq!(decoder.parameters(), (&before.0, &before.1));
                    if let Some(frame) = decoder.decode_packet(&packet).unwrap() {
                        assert!(!headers.is_empty());
                        pictures += 1;
                        let wide = frame.picture.depth.into_iter().max().unwrap() > 8;
                        for plane in &frame.picture.planes {
                            for &sample in plane.samples() {
                                if wide {
                                    actual.extend_from_slice(&sample.to_le_bytes());
                                } else {
                                    actual.push(sample as u8);
                                }
                            }
                        }
                    } else {
                        assert!(headers.is_empty());
                    }
                }
                assert_eq!(
                    pictures,
                    case["old_frames"].as_u64().unwrap() + case["new_frames"].as_u64().unwrap()
                );
                assert_eq!(actual, expected, "{name}, inband={inband}");
                decoder.reset();
                assert_eq!(decoder.parameters().0, &initial);
            }
        }
    }
}
