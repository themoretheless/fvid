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
        assert_eq!(
            frames,
            reference.len() / (128 * 128 * 3 / 2 * if depth == 8 { 1 } else { 2 })
        );
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

#[test]
fn multislice_temporal_stream_matches_reference_across_reference_storage() {
    compare_multislice(
        include_bytes!("fixtures/playback-errors/hevc-multislice-temporal.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-multislice-temporal.yuv"),
        8,
    );
    let mut input = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-temporal.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let track = input.tracks()[0].clone();
    let decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    let mut temporal = false;
    let mut multiple_references = false;
    for index in 0..track.samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        for header in decoder.slice_headers(&packet).unwrap() {
            temporal |= header.temporal_mvp;
            multiple_references |= header.references.iter().any(|&count| count > 1);
        }
    }
    assert!(temporal && multiple_references);
}

#[test]
fn temporal_multislice_seek_rebuilds_reference_storage_after_eof() {
    let source = include_bytes!("fixtures/playback-errors/hevc-multislice-temporal.mp4");
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(source), Default::default(), 16 << 20)
            .unwrap();
    let pixels = |frame: &fvid::playback_mp4::VideoFrame| {
        [
            frame.picture.y.as_slice(),
            frame.picture.cb.as_slice(),
            frame.picture.cr.as_slice(),
        ]
        .concat()
    };
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        frames.push((frame.presentation_time.ticks, pixels(&frame)));
    }
    assert_eq!(frames.len(), 18);
    for index in [10, 4, 17, 0, 13] {
        let (target, expected) = &frames[index];
        reader.seek_to_sync(*target);
        let actual = loop {
            let frame = reader.read_frame().unwrap().expect("seek target exists");
            if frame.presentation_time.ticks >= *target {
                break frame;
            }
        };
        assert_eq!(actual.presentation_time.ticks, *target);
        assert_eq!(pixels(&actual), *expected);
    }
}

#[test]
fn dependent_segments_match_hm_and_independent_decoder() {
    let source = include_bytes!("fixtures/playback-errors/hevc-dependent-main.mp4");
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let track = input.tracks()[0].clone();
    let decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    assert!(decoder.parameters().1.dependent_slices);
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 2);
        assert!(!headers[0].dependent && headers[1].dependent);
        assert_eq!(headers[0].qp, headers[1].qp);
    }
    compare_multislice(
        source,
        include_bytes!("fixtures/playback-errors/hevc-dependent-main.yuv"),
        8,
    );
}

#[test]
fn dependent_wpp_segments_cross_rows_and_match_both_oracles() {
    let source = include_bytes!("fixtures/playback-errors/hevc-dependent-wpp.mp4");
    let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let track = input.tracks()[0].clone();
    let decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    assert!(decoder.parameters().1.entropy_sync);
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 4);
        assert!(headers.iter().skip(1).all(|h| h.dependent));
        assert_eq!(
            headers.iter().map(|h| h.address).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }
    compare_multislice(
        source,
        include_bytes!("fixtures/playback-errors/hevc-dependent-wpp.yuv"),
        8,
    );
}

#[test]
fn dependent_inter_segments_match_saved_reference() {
    compare_multislice(
        include_bytes!("fixtures/playback-errors/hevc-dependent-inter.mp4"),
        include_bytes!("fixtures/playback-errors/hevc-dependent-inter.yuv"),
        8,
    );
}
