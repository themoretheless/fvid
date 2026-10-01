use fvid::{
    codec::{
        avc_decoder::AvcDecoder,
        config::{AvcConfig, NalUnits},
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
const VIDEO: &[u8] = include_bytes!("fixtures/playback-errors/avc-inband-resize.mp4");
#[test]
fn repeated_and_changed_in_band_parameters_match_oracle_and_reset() {
    let mut source = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let track = source.tracks()[0].clone();
    assert_eq!(track.codec, *b"avc3");
    let length = AvcConfig::parse(&track.configuration).unwrap().length_size;
    let mut decoder = AvcDecoder::new(&track.configuration, 16 << 20).unwrap();
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut sizes = Vec::new();
        let mut repeats = 0;
        let mut packet = Vec::new();
        for index in 0..track.samples.len() {
            source.read_packet(0, index, &mut packet).unwrap();
            let kinds: Vec<_> = NalUnits::new(&packet, length)
                .unwrap()
                .map(|nal| nal.unwrap()[0] & 31)
                .collect();
            if kinds.contains(&7) {
                assert!(kinds.contains(&8));
                repeats += 1;
            }
            let picture = decoder.decode_order(&packet).unwrap().unwrap();
            sizes.push([picture.coded_width, picture.coded_height]);
            actual.extend(
                picture
                    .y
                    .iter()
                    .chain(&picture.cb)
                    .chain(&picture.cr)
                    .map(|v| *v as u8),
            );
        }
        assert_eq!(repeats, 2);
        assert_eq!(
            sizes,
            [[64, 64], [64, 64], [64, 64], [96, 64], [96, 64], [96, 64]]
        );
        assert_eq!(
            actual,
            include_bytes!("fixtures/playback-errors/avc-inband-resize.yuv")
        );
        decoder.reset();
    }
}

fn split_parameters(packet: &[u8], length: u8) -> (Vec<u8>, Vec<u8>) {
    let mut parameters = Vec::new();
    let mut picture = Vec::new();
    for nal in NalUnits::new(packet, length).unwrap() {
        let nal = nal.unwrap();
        let output = if matches!(nal[0] & 31, 7 | 8) {
            &mut parameters
        } else {
            &mut picture
        };
        output.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        output.extend_from_slice(nal);
    }
    (parameters, picture)
}

#[test]
fn parameter_only_update_persists_and_invalid_transition_requires_reset() {
    let mut source = Mp4Reader::open(Cursor::new(VIDEO), Default::default()).unwrap();
    let config = source.tracks()[0].configuration.clone();
    let length = AvcConfig::parse(&config).unwrap().length_size;
    let mut first = Vec::new();
    source.read_packet(0, 0, &mut first).unwrap();
    let mut changed = Vec::new();
    source.read_packet(0, 3, &mut changed).unwrap();
    let (parameters, picture) = split_parameters(&changed, length);
    let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
    decoder.decode_order(&first).unwrap().unwrap();
    assert!(decoder.decode_order(&parameters).unwrap().is_none());
    let decoded = decoder.decode_order(&picture).unwrap().unwrap();
    assert_eq!([decoded.coded_width, decoded.coded_height], [96, 64]);
    let bytes: Vec<_> = decoded
        .y
        .iter()
        .chain(&decoded.cb)
        .chain(&decoded.cr)
        .map(|v| *v as u8)
        .collect();
    assert_eq!(
        bytes,
        &include_bytes!("fixtures/playback-errors/avc-inband-resize.yuv")
            [3 * 64 * 64 * 3 / 2..3 * 64 * 64 * 3 / 2 + 96 * 64 * 3 / 2]
    );
    decoder.reset();
    let mut inter = Vec::new();
    source.read_packet(0, 4, &mut inter).unwrap();
    for (input, expected) in [
        (
            [first.as_slice(), &parameters].concat(),
            "AVC changed SPS follows picture slices",
        ),
        (
            [parameters.as_slice(), &inter].concat(),
            "AVC stream/configuration change requires IDR",
        ),
    ] {
        decoder.decode_order(&first).unwrap().unwrap();
        let error = match decoder.decode_order(&input) {
            Err(error) => error,
            Ok(_) => panic!("invalid transition accepted"),
        };
        assert!(error.to_string().contains(expected), "{error}");
        assert!(decoder.decode_order(&first).is_err());
        decoder.reset();
    }
    assert_eq!(
        decoder.decode_order(&first).unwrap().unwrap().coded_width,
        64
    );
}

#[test]
fn native_reader_and_camera_keep_working_across_avc3_resize_and_rewind() {
    use fvid::{
        playback_native::NativeReader,
        virtual_camera::{CameraTick, LatestFrame, NativeCameraSource},
    };
    let mut reader = NativeReader::software(Cursor::new(VIDEO), 16 << 20).unwrap();
    let mut positions = Vec::new();
    let mut sizes = Vec::new();
    while reader.read_frame().unwrap() {
        sizes.push(reader.dimensions());
        let (start, _, scale) = reader.frame_interval().unwrap();
        positions.push((start * 1_000_000_000).div_ceil(u128::from(scale)) as u64);
    }
    assert_eq!(
        sizes,
        [[64, 64], [64, 64], [64, 64], [96, 64], [96, 64], [96, 64]]
    );
    let mut source =
        NativeCameraSource::new(NativeReader::software(Cursor::new(VIDEO), 16 << 20).unwrap());
    let output = LatestFrame::new(64, 64, 64 * 64 * 4).unwrap();
    let mut pixels = vec![0; 64 * 64 * 4];
    let mut first = Vec::new();
    for (sequence, position) in positions.iter().chain(positions.iter().take(1)).enumerate() {
        let tick = CameraTick {
            sequence: sequence as u64,
            host_time_ns: sequence as u64 + 1,
            media_time_ns: *position,
        };
        assert!(source.publish(tick, &output).unwrap());
        assert_eq!(output.copy_latest(None, &mut pixels).unwrap(), Some(tick));
        if sequence == 0 {
            first = pixels.clone();
        }
        if sequence == sizes.len() {
            assert_eq!(pixels, first);
        }
    }
}
