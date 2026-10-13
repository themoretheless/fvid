//! Owned ISOBMFF final-OBU framing regression, no fixture generation in tests.
use fvid::{
    codec::{av1::Obus, av1_decoder::Decoder},
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 32 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-implicit-size.json")).unwrap()).unwrap()
}
fn pixels(frame: &fvid::playback::video::mp4::VideoFrame) -> Vec<u8> {
    let planes = frame.planes8.as_ref().unwrap();
    planes
        .y
        .iter()
        .chain(&planes.cb)
        .chain(&planes.cr)
        .copied()
        .collect()
}
#[test]
fn implicit_final_obus_match_saved_pixels_timestamps_seek_and_rewind() {
    let expected = std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap();
    for case in manifest()["cases"].as_array().unwrap() {
        if case["error"].is_string() {
            continue;
        }
        let file = case["file"].as_str().unwrap();
        let data = std::fs::read(root().join(file)).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Limits::default(), BUDGET)
                .unwrap();
        let metadata = case["last_kind"] == 5;
        if metadata {
            assert_eq!(reader.bitstream_hdr().light.max_cll, 1234.0);
            assert_eq!(reader.bitstream_hdr().light.max_fall, 567.0);
        }
        for _ in 0..2 {
            let frame = reader.read_frame().unwrap().unwrap();
            assert!(pixels(&frame) == expected, "{file}");
            assert_eq!(frame.sample_index, 0);
            assert_eq!(frame.presentation_time.ticks, 0);
            assert_eq!(frame.duration.ticks, 1);
            assert!(reader.read_frame().unwrap().is_none());
            assert_eq!(reader.seek_to_sync(0), 0);
        }
        reader.rewind();
        assert!(pixels(&reader.read_frame().unwrap().unwrap()) == expected);
        let mut input = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        let mut decoder =
            Decoder::from_configuration(&input.tracks()[0].configuration, BUDGET).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let obus = Obus::from_container_sample(&packet)
            .collect::<fvid::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            obus.last().unwrap().kind,
            case["last_kind"].as_u64().unwrap() as u8
        );
        assert_eq!(
            Obus::new(&packet)
                .find_map(Result::err)
                .unwrap()
                .to_string(),
            "AV1 low-overhead OBU requires a size field"
        );
        // Raw APIs remain strict; reset then allow only the demuxed sample API.
        assert_eq!(
            decoder
                .decode_temporal_unit(&packet)
                .err()
                .unwrap()
                .to_string(),
            "AV1 low-overhead OBU requires a size field"
        );
        decoder.reset();
        for _ in 0..2 {
            let frame = decoder.decode_container_sample(&packet).unwrap().unwrap();
            let actual: Vec<u8> = frame
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|&v| v as u8))
                .collect();
            assert!(actual == expected, "{file}: direct pixels");
            decoder.finish().unwrap();
            if metadata {
                assert_eq!(decoder.hdr().light.max_cll, 1234.0);
            }
            decoder.reset();
        }
    }
}
#[test]
fn implicit_sizes_do_not_relax_extensions_or_configuration() {
    for case in manifest()["cases"].as_array().unwrap() {
        let Some(error) = case["error"].as_str() else {
            continue;
        };
        let file = case["file"].as_str().unwrap();
        let data = std::fs::read(root().join(file)).unwrap();
        if case["configuration"] == true {
            assert_eq!(
                Mp4VideoReader::open_software(Cursor::new(data), Limits::default(), BUDGET)
                    .err()
                    .expect("invalid config accepted")
                    .to_string(),
                error
            );
            continue;
        }
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Limits::default(), BUDGET)
                .unwrap();
        assert_eq!(
            reader
                .read_frame()
                .err()
                .expect("invalid header accepted")
                .to_string(),
            error
        );
        let mut input = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        let mut decoder =
            Decoder::from_configuration(&input.tracks()[0].configuration, BUDGET).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        for _ in 0..2 {
            assert_eq!(
                decoder
                    .decode_container_sample(&packet)
                    .err()
                    .unwrap()
                    .to_string(),
                error
            );
            assert_eq!(
                decoder
                    .decode_container_sample(&[0x10])
                    .err()
                    .unwrap()
                    .to_string(),
                "AV1 decoder requires reset after error"
            );
            decoder.reset();
        }
    }
}

#[test]
fn webm_implicit_final_obus_match_pixels_metadata_and_media_library() {
    let expected = std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap();
    for case in manifest()["webm_cases"].as_array().unwrap() {
        if case["error"].is_string() {
            continue;
        }
        let file = case["file"].as_str().unwrap();
        let mut reader = fvid::playback_webm::WebmVideoReader::open(
            Cursor::new(std::fs::read(root().join(file)).unwrap()),
            BUDGET,
        )
        .unwrap();
        if case["last_kind"] == 5 {
            assert_eq!(reader.bitstream_hdr().light.max_cll, 1234.0);
            assert_eq!(reader.bitstream_hdr().light.max_fall, 567.0);
        }
        for replay in 0..3 {
            let fvid::playback_native::RawFrame::Planar8(frame) =
                reader.read_frame_raw().unwrap().unwrap()
            else {
                panic!("expected 8-bit planar frame");
            };
            let pixels: Vec<u8> = frame
                .y
                .iter()
                .chain(&frame.cb)
                .chain(&frame.cr)
                .copied()
                .collect();
            assert!(pixels == expected, "{file}");
            assert_eq!(
                reader.frame_interval(),
                Some((0, 20_000_000, 1_000_000_000))
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
            if replay == 0 {
                reader.rewind();
            } else if replay == 1 {
                assert_eq!(reader.seek_to_sync(0).unwrap(), 0);
            }
        }
        let stats = fvid_media::decode_video(&root().join(file)).unwrap();
        assert_eq!(stats.video_frames, 1, "{file}");
        assert_eq!((stats.width, stats.height), (128, 128));
        assert_eq!(stats.decode_errors, 0);
    }
}
#[test]
fn webm_implicit_sizes_keep_header_and_private_configuration_refusals() {
    for case in manifest()["webm_cases"].as_array().unwrap() {
        let Some(error) = case["error"].as_str() else {
            continue;
        };
        let file = case["file"].as_str().unwrap();
        let data = std::fs::read(root().join(file)).unwrap();
        if case["configuration"] == true {
            assert_eq!(
                fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), BUDGET)
                    .err()
                    .expect("invalid private accepted")
                    .to_string(),
                error
            );
        } else {
            let mut reader =
                fvid::playback_webm::WebmVideoReader::open(Cursor::new(data), BUDGET).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    reader
                        .read_frame_raw()
                        .err()
                        .expect("invalid extension accepted")
                        .to_string(),
                    error
                );
                reader.rewind();
            }
        }
        assert_eq!(
            fvid_media::decode_video(&root().join(file)).unwrap_err(),
            error
        );
    }
}
