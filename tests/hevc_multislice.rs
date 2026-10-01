use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::io::Cursor;

#[test]
fn independent_multislice_picture_headers_have_ordered_shared_identity() {
    let mut input = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let track = input.tracks()[0].clone();
    let decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    assert_eq!(track.samples.len(), 3);
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 2);
        assert!(headers.iter().all(|h| h.entropy_substreams.len() == 2));
        assert!(headers.iter().any(|h| h.sao != [false, false]));
        assert!(headers.iter().all(|h| !h.deblocking.disabled));
        assert!(headers[0].first);
        assert!(!headers[1].first);
        assert_eq!((headers[0].address, headers[1].address), (0, 8));
        assert_eq!(headers[0].poc_lsb, headers[1].poc_lsb);
        // Appending a second copy starts a new picture inside this access unit.
        let mut duplicate = packet.clone();
        duplicate.extend_from_slice(&packet);
        assert!(decoder.slice_headers(&duplicate).is_err());
        for cut in 1..packet.len() {
            // A prefix ending exactly at a NAL boundary may be one valid slice;
            // every other truncation must be a checked error, never a panic.
            let _ = decoder.slice_headers(&packet[..cut]);
        }
    }
    assert_eq!(
        include_bytes!("fixtures/playback-errors/hevc-multislice-main.yuv").len(),
        3 * 128 * 128 * 3 / 2
    );
}

fn compare_multislice(source: &[u8], reference: &[u8], depth: u8) {
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(source), Default::default(), 16 << 20)
            .unwrap();
    assert!(!reader.hardware_accelerated());
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut frames = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            for plane in [&frame.picture.y, &frame.picture.cb, &frame.picture.cr] {
                for &value in plane {
                    if depth == 8 {
                        actual.push(value as u8);
                    } else {
                        actual.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            frames += 1;
        }
        assert_eq!(frames, 3);
        let mismatches: Vec<_> = actual
            .iter()
            .zip(reference)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .collect();
        assert_eq!(actual.len(), reference.len());
        assert!(mismatches.is_empty(), "{mismatches:?}");
        reader.rewind();
    }
}

#[test]
fn multislice_main_matches_independent_yuv_and_rewind() {
    compare_multislice(
        include_bytes!("fixtures/playback-errors/hevc-multislice-main.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-multislice-main.yuv"),
        8,
    );
}
#[test]
fn multislice_main10_matches_independent_yuv_and_rewind() {
    compare_multislice(
        include_bytes!("fixtures/playback-errors/hevc-multislice-main10.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-multislice-main10.yuv"),
        10,
    );
}

#[test]
fn incomplete_multislice_picture_is_never_published_and_reset_recovers() {
    let source = include_bytes!("fixtures/playback-errors/hevc-multislice-main.mp4");
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let track = input.tracks()[0].clone();
    let mut decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    for cut in 1..packet.len() {
        decoder.reset();
        assert!(
            !matches!(decoder.decode_packet(&packet[..cut]), Ok(Some(_))),
            "cut {cut}"
        );
    }
    decoder.reset();
    assert!(decoder.decode_packet(&packet[..packet.len() - 1]).is_err());
    assert!(decoder.decode_packet(&packet).is_err());
    decoder.reset();
    assert!(decoder.decode_packet(&packet).unwrap().is_some());
}
