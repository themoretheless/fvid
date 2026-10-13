//! Owned av1C initialization and structural validation; offline saved pixels.
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
    serde_json::from_slice(&std::fs::read(root().join("av1-configuration.json")).unwrap()).unwrap()
}
fn movie(file: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    Mp4Reader::open(
        Cursor::new(std::fs::read(root().join(file)).unwrap()),
        Limits::default(),
    )
    .unwrap()
}
fn frame_pixels(frame: &fvid::playback::video::mp4::VideoFrame) -> Vec<u8> {
    let planes = frame.planes8.as_ref().expect("8-bit AV1 display planes");
    planes
        .y
        .iter()
        .chain(&planes.cb)
        .chain(&planes.cr)
        .copied()
        .collect()
}
#[test]
fn valid_configuration_only_sequence_delay_and_inband_controls_match_pixels() {
    let expected = std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap();
    for case in manifest()["cases"].as_array().unwrap() {
        if !case["error"].is_null() {
            continue;
        }
        let file = case["file"].as_str().unwrap();
        let bytes = std::fs::read(root().join(file)).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), BUDGET).unwrap();
        for _ in 0..2 {
            let frame = reader.read_frame().unwrap().unwrap();
            assert_eq!(frame.sample_index, 0);
            assert_eq!(frame.presentation_time.ticks, 0);
            assert_eq!(frame.duration.ticks, 1);
            assert!(frame_pixels(&frame) == expected, "{file}: pixels");
            assert!(reader.read_frame().unwrap().is_none());
            assert_eq!(reader.seek_to_sync(0), 0);
        }
        reader.rewind();
        assert!(frame_pixels(&reader.read_frame().unwrap().unwrap()) == expected);
        let mut input = movie(file);
        assert_eq!(
            input.tracks()[0].samples.get(0).unwrap().sync,
            file.contains("inband")
        );
        let config = input.tracks()[0].configuration.clone();
        if let Some(obu) = Obus::new(&config[4..])
            .map(Result::unwrap)
            .find(|o| o.kind == 1)
        {
            let sequence = fvid::codec::av1_sequence::Sequence::parse(obu.payload).unwrap();
            assert_eq!(
                config[1],
                (sequence.profile << 5) | sequence.operating_points[0].level
            );
            let color = &sequence.color;
            let hints = (u8::from(sequence.operating_points[0].tier) << 7)
                | (u8::from(color.depth > 8) << 6)
                | (u8::from(color.depth == 12) << 5)
                | (u8::from(color.monochrome) << 4)
                | (u8::from(color.subsampling[0]) << 3)
                | (u8::from(color.subsampling[1]) << 2)
                | color.chroma_position;
            assert_eq!(config[2], hints);
        }
        let mut decoder = Decoder::from_configuration(&config, BUDGET).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        if !file.contains("inband") {
            assert!(Obus::new(&packet).all(|o| o.unwrap().kind != 1));
        }
        for _ in 0..2 {
            let frames = decoder.decode_packet(&packet).unwrap();
            let pixels: Vec<u8> = frames
                .iter()
                .filter(|f| f.show)
                .flat_map(|f| {
                    f.picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples.iter().map(|&v| v as u8))
                })
                .collect();
            assert!(pixels == expected, "{file}: direct pixels");
            decoder.finish().unwrap();
            decoder.reset();
        }
    }
}
#[test]
fn invalid_configuration_structure_is_refused_by_decoder_and_player() {
    for case in manifest()["cases"].as_array().unwrap() {
        let Some(error) = case["error"].as_str() else {
            continue;
        };
        let file = case["file"].as_str().unwrap();
        let input = movie(file);
        let config = input.tracks()[0].configuration.clone();
        if case["inband"] == true {
            let mut decoder = Decoder::from_configuration(&config, BUDGET).unwrap();
            let mut input = movie(file);
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            for _ in 0..2 {
                let result = decoder
                    .decode_packet(&packet)
                    .err()
                    .expect("mismatch accepted");
                assert!(matches!(result, fvid::Error::Invalid(_)));
                assert_eq!(result.to_string(), error, "{file}");
                decoder.reset();
            }
            let mut reader = Mp4VideoReader::open_software(
                Cursor::new(std::fs::read(root().join(file)).unwrap()),
                Limits::default(),
                BUDGET,
            )
            .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    reader
                        .read_frame()
                        .err()
                        .expect("mismatch accepted")
                        .to_string(),
                    error,
                    "{file}"
                );
                reader.rewind();
            }
            continue;
        }
        if !config.is_empty() {
            let result = Decoder::from_configuration(&config, BUDGET)
                .err()
                .expect("invalid configuration accepted");
            assert_eq!(result.to_string(), error, "{file}");
            if file.contains("version") {
                assert!(matches!(result, fvid::Error::Unsupported(_)));
            } else {
                assert!(matches!(result, fvid::Error::Invalid(_)));
            }
        }
        let result = Mp4VideoReader::open_software(
            Cursor::new(std::fs::read(root().join(file)).unwrap()),
            Limits::default(),
            BUDGET,
        )
        .err()
        .expect("invalid MP4 configuration accepted");
        assert_eq!(result.to_string(), error, "{file}");
    }
    assert!(Decoder::from_configuration(&[], BUDGET).is_ok());
}
