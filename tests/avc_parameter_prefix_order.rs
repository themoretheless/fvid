use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        config::AvcConfig,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-parameter-prefix-order")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let cases: Vec<_> = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    assert_eq!(cases.len(), 12);
    cases
}
#[test]
fn pcm_controls_prove_pps_context_and_all_component_samples() {
    for stem in ["y8-c8", "y10-c12", "y14-c8"] {
        let old_cfg = bytes(&format!("{stem}-old.avcc"));
        let new_cfg = bytes(&format!("{stem}-new.avcc"));
        let old = AvcConfig::parse(&old_cfg).unwrap();
        let new = AvcConfig::parse(&new_cfg).unwrap();
        let old_sps = Sps::parse(old.sps[0]).unwrap();
        let new_sps = Sps::parse(new.sps[0]).unwrap();
        assert_eq!(old_sps.id, new_sps.id);
        assert_eq!(old_sps.chroma_format, 0);
        assert_eq!(new_sps.chroma_format, 3);
        assert!(Pps::parse(new.pps[0], &old_sps).is_err());
        let pps = Pps::parse(new.pps[0], &new_sps).unwrap();
        assert!(pps.transform_8x8);
        assert_eq!(pps.scaling_lists.unwrap().len(), 12);
        let mut source = Mp4Reader::open(
            Cursor::new(bytes(&format!("{stem}-sps-pps-inline.mp4"))),
            Limits::default(),
        )
        .unwrap();
        for (label, start, cfg) in [("old", 0, &old_cfg), ("new", 2, &new_cfg)] {
            let mut decoder = AvcDecoder::new(cfg, BUDGET).unwrap();
            let mut actual = Vec::new();
            for i in start..start + 2 {
                let mut packet = Vec::new();
                source.read_packet(0, i, &mut packet).unwrap();
                decoder
                    .decode_order(&packet)
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual, bytes(&format!("{stem}-{label}.yuv")));
        }
    }
}
#[test]
fn avc3_reordered_parameter_prefix_matches_pixels_timing_seek_and_rewind() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let stem = case["stem"].as_str().unwrap();
        let prefix = usize::from(case["separate"].as_bool().unwrap());
        let mut reader = Mp4VideoReader::open_software(
            Cursor::new(bytes(&format!("{name}.mp4"))),
            Limits::default(),
            BUDGET,
        )
        .unwrap();
        for pass in 0..3 {
            if pass == 1 {
                reader.rewind();
            }
            if pass == 2 {
                assert_eq!(
                    reader.seek_to_sync((2 + prefix) as i64),
                    (2 + prefix) as i64
                );
            }
            let mut old = Vec::new();
            let mut new = Vec::new();
            for i in if pass == 2 { 2 } else { 0 }..4 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name}/{pass}/{i}: {e}"))
                    .unwrap();
                let index = i + if i >= 2 { prefix } else { 0 };
                assert_eq!(frame.sample_index, index);
                assert_eq!(frame.presentation_time.ticks, index as i64);
                assert_eq!(
                    frame.duration.nanoseconds().unwrap(),
                    if prefix == 1 && i == 1 {
                        80_000_000
                    } else {
                        40_000_000
                    }
                );
                assert_eq!(
                    frame.picture.chroma_format().unwrap(),
                    if i < 2 { 0 } else { 3 }
                );
                assert_eq!(
                    frame.picture.bit_depth,
                    case["ydepth"].as_u64().unwrap() as u8
                );
                assert_eq!(
                    frame.picture.chroma_bit_depth,
                    case["cdepth"].as_u64().unwrap() as u8
                );
                frame
                    .picture
                    .write_planar(if i < 2 { &mut old } else { &mut new })
                    .unwrap();
            }
            if pass < 2 {
                assert_eq!(old, bytes(&format!("{stem}-old.yuv")), "{name}");
            }
            assert_eq!(new, bytes(&format!("{stem}-new.yuv")), "{name}");
            assert!(reader.read_frame().unwrap().is_none());
        }
    }
}

#[test]
fn direct_avc_parameter_replacement_matches_all_samples_and_reset() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let stem = case["stem"].as_str().unwrap();
        let mut expected = bytes(&format!("{stem}-old.yuv"));
        expected.extend(bytes(&format!("{stem}-new.yuv")));
        let mut source = Mp4Reader::open(
            Cursor::new(bytes(&format!("{name}.mp4"))),
            Limits::default(),
        )
        .unwrap();
        let mut decoder = AvcDecoder::new(&source.tracks()[0].configuration, BUDGET).unwrap();
        for pass in 0..2 {
            let mut actual = Vec::new();
            let mut pictures = 0;
            for i in 0..source.tracks()[0].samples.len() {
                let mut packet = Vec::new();
                source.read_packet(0, i, &mut packet).unwrap();
                if let Some(picture) = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name}/{pass}/{i}: {e}"))
                {
                    picture.write_planar(&mut actual).unwrap();
                    pictures += 1;
                } else {
                    assert!(case["separate"].as_bool().unwrap() && i == 2);
                }
            }
            assert_eq!(pictures, 4);
            assert_eq!(actual, expected, "{name}");
            decoder.reset();
        }
    }
}

#[test]
fn priming_restores_latest_bank_without_pictures_and_requires_reset_after_error() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let stem = case["stem"].as_str().unwrap();
        let prefix = usize::from(case["separate"].as_bool().unwrap());
        let mut source = Mp4Reader::open(
            Cursor::new(bytes(&format!("{name}.mp4"))),
            Limits::default(),
        )
        .unwrap();
        let config = source.tracks()[0].configuration.clone();
        let mut decoder = AvcDecoder::new(&config, BUDGET).unwrap();
        let mut first = Vec::new();
        source.read_packet(0, 0, &mut first).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            for i in 0..2 + prefix {
                source.read_packet(0, i, &mut packet).unwrap();
                decoder.prime_parameters(&packet).unwrap();
            }
            let mut actual = Vec::new();
            for i in 2 + prefix..4 + prefix {
                source.read_packet(0, i, &mut packet).unwrap();
                decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name}/{pass}/{i}: {e}"))
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual, bytes(&format!("{stem}-new.yuv")));
            assert!(
                decoder
                    .prime_parameters(&first)
                    .unwrap_err()
                    .to_string()
                    .contains("requires a reset decoder")
            );
            decoder.reset();
        }
        assert!(
            decoder
                .prime_parameters(&bytes("truncated-pps.packet"))
                .is_err()
        );
        assert!(
            decoder
                .prime_parameters(&first)
                .unwrap_err()
                .to_string()
                .contains("requires reset")
        );
        assert!(
            decoder
                .decode_order(&first)
                .unwrap_err()
                .to_string()
                .contains("requires reset")
        );
        decoder.reset();
        let mut actual = Vec::new();
        for i in 0..2 {
            source.read_packet(0, i, &mut packet).unwrap();
            decoder
                .decode_order(&packet)
                .unwrap()
                .unwrap()
                .write_planar(&mut actual)
                .unwrap();
        }
        assert_eq!(actual, bytes(&format!("{stem}-old.yuv")));
        let mut tiny = AvcDecoder::new(&config, 4).unwrap();
        assert!(
            tiny.prime_parameters(&bytes("truncated-pps.packet"))
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
        assert!(
            tiny.prime_parameters(&[])
                .unwrap_err()
                .to_string()
                .contains("requires reset")
        );
    }
}
