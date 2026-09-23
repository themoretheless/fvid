use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader, playback_native::NativeReader};
use std::{io::Cursor, time::Duration};
fn compare(mp4: &[u8], yuv: &[u8], depth: u8) {
    let mut source = Mp4VideoReader::open(Cursor::new(mp4), Limits::default(), 16 << 20).unwrap();
    assert!(!source.hardware_accelerated());
    for pass in 0..2 {
        let mut bytes = Vec::new();
        let mut count = 0;
        let mut end = None;
        while let Some(frame) = source.read_frame().unwrap() {
            assert_eq!(frame.picture.bit_depth, depth);
            if let Some(previous) = end {
                assert_eq!(frame.presentation_time.ticks, previous);
            }
            end = Some(frame.presentation_time.ticks + frame.duration.ticks);
            for plane in [&frame.picture.y, &frame.picture.cb, &frame.picture.cr] {
                for &v in plane {
                    if depth == 8 {
                        bytes.push(v as u8);
                    } else {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            count += 1;
        }
        assert_eq!(count, 17);
        assert_eq!(bytes.len(), yuv.len());
        let diffs: Vec<_> = bytes
            .iter()
            .zip(yuv)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(10)
            .collect();
        assert!(diffs.is_empty(), "pass {pass}: {diffs:?}");
        source.rewind();
    }
}
#[test]
fn main_ipb_wpp_qp_filters_match_oracle() {
    compare(
        include_bytes!("fixtures/hevc/main-ipb.mp4"),
        include_bytes!("fixtures/hevc/main-ipb.yuv"),
        8,
    );
}
#[test]
fn main10_ipb_wpp_qp_filters_match_oracle() {
    compare(
        include_bytes!("fixtures/hevc/main10-ipb.mp4"),
        include_bytes!("fixtures/hevc/main10-ipb.yuv"),
        10,
    );
}
#[test]
fn weighted_temporal_prediction_matches_oracle() {
    compare(
        include_bytes!("fixtures/hevc/weighted-tmvp.mp4"),
        include_bytes!("fixtures/hevc/weighted-tmvp.yuv"),
        8,
    );
}
#[test]
fn native_dispatch_rewind_and_seek() {
    let mut reader = NativeReader::without_memory_limit(Cursor::new(include_bytes!(
        "fixtures/hevc/main-ipb.mp4"
    )))
    .unwrap();
    assert!(!reader.hardware_accelerated());
    assert!(reader.seekable());
    assert!(reader.read_frame().unwrap());
    assert_eq!(reader.dimensions(), [128, 128]);
    let first = reader.rgb().to_vec();
    reader.seek(Duration::from_millis(350)).unwrap();
    let (start, end, scale) = reader.frame_interval().unwrap();
    assert!(start * 1000 <= 350 * u128::from(scale) && end * 1000 > 350 * u128::from(scale));
    reader.rewind().unwrap();
    assert!(reader.read_frame().unwrap());
    assert_eq!(reader.rgb(), first);
}

#[test]
fn open_gop_seek_discards_leading_rasl_and_matches_sequential_frames() {
    let file = include_bytes!("fixtures/hevc/weighted-tmvp.mp4");
    let mut reader = NativeReader::without_memory_limit(Cursor::new(file)).unwrap();
    let mut frames = Vec::new();
    while reader.read_frame().unwrap() {
        frames.push((reader.frame_interval().unwrap(), reader.rgb().to_vec()));
    }
    for millis in [350, 267, 533, 100, 0] {
        reader.seek(Duration::from_millis(millis)).unwrap();
        let interval = reader.frame_interval().unwrap();
        let expected = frames.iter().find(|(i, _)| *i == interval).unwrap();
        assert_eq!(reader.rgb(), expected.1, "seek {millis}");
    }
}

#[test]
fn fixture_exercises_temporal_weights_and_open_gop() {
    use fvid::codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::HevcDecoder,
        hevc_slice::SliceHeader,
    };
    use fvid::container::mp4::Mp4Reader;
    let mut reader = Mp4Reader::open(
        Cursor::new(include_bytes!("fixtures/hevc/weighted-tmvp.mp4")),
        Limits::default(),
    )
    .unwrap();
    let track = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let config = &reader.tracks()[track].configuration;
    let length = HevcConfig::parse(config).unwrap().length_size;
    let decoder = HevcDecoder::from_configuration(config, 16 << 20).unwrap();
    let (sps, pps) = decoder.parameters();
    let mut temporal = false;
    let mut weighted = false;
    let mut cra = false;
    let mut rasl = false;
    let mut wpp = false;
    let mut packet = Vec::new();
    for sample in 0..reader.tracks()[track].samples.len() {
        reader.read_packet(track, sample, &mut packet).unwrap();
        for nal in NalUnits::new(&packet, length).unwrap() {
            let nal = nal.unwrap();
            if (nal[0] >> 1) & 63 >= 32 {
                continue;
            }
            let h = SliceHeader::parse(nal, sps, pps, 1 << 20).unwrap();
            temporal |= h.temporal_mvp;
            cra |= h.nal.unit_type == 21;
            rasl |= matches!(h.nal.unit_type, 8 | 9);
            wpp |= !h.entry_point_offsets.is_empty();
            if let Some(w) = h.weights {
                weighted |= w.lists.iter().flatten().any(|v| {
                    (0..3).any(|c| {
                        v.offsets[c] != 0
                            || v.values[c] != (1 << w.denominators[usize::from(c != 0)])
                    })
                });
            }
        }
    }
    assert!(temporal && weighted && cra && rasl && wpp);
    assert!(pps.cu_qp_delta_depth.is_some());
}

#[test]
fn malformed_packets_poison_until_reset_and_small_budgets_fail() {
    use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
    let mut source = Mp4Reader::open(
        Cursor::new(include_bytes!("fixtures/hevc/main-ipb.mp4")),
        Limits::default(),
    )
    .unwrap();
    let track = source
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let config = source.tracks()[track].configuration.clone();
    let mut packet = Vec::new();
    source.read_packet(track, 0, &mut packet).unwrap();
    for end in 1..64 {
        let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
        assert!(decoder.decode_packet(&packet[..end]).is_err());
        assert!(decoder.decode_packet(&packet).is_err());
        decoder.reset();
        assert!(decoder.decode_packet(&packet).unwrap().is_some());
    }
    let mut decoder = HevcDecoder::from_configuration(&config, 8192).unwrap();
    assert!(decoder.decode_packet(&packet).is_err());
}

#[test]
fn truncated_final_row_closes_reconstruction_worker() {
    use fvid::{
        codec::{
            config::{HevcConfig, NalUnits},
            hevc_decoder::HevcDecoder,
            hevc_picture,
            hevc_slice::SliceHeader,
        },
        container::mp4::Mp4Reader,
    };
    let mut source = Mp4Reader::open(
        Cursor::new(include_bytes!("fixtures/hevc/main-ipb.mp4")),
        Limits::default(),
    )
    .unwrap();
    let track = source
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let config = source.tracks()[track].configuration.clone();
    let length = HevcConfig::parse(&config).unwrap().length_size;
    let decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let (sps, pps) = decoder.parameters();
    assert!(!pps.constrained_intra);
    let mut packet = Vec::new();
    source.read_packet(track, 0, &mut packet).unwrap();
    let nal = NalUnits::new(&packet, length)
        .unwrap()
        .map(Result::unwrap)
        .find(|nal| (nal[0] >> 1) & 63 < 32)
        .unwrap();
    let mut slice = SliceHeader::parse(nal, sps, pps, 1 << 20).unwrap();
    assert!(slice.entropy_substreams.len() > 2);
    let last = slice.entropy_substreams.last_mut().unwrap();
    last.end = last.start + 2;
    slice.rbsp.truncate(last.end);
    assert!(hevc_picture::decode_idr(sps, pps, &slice, 16 << 20).is_err());
}

#[test]
fn raw_seek_after_eof_matches_sequential_pixels_and_keeps_planes() {
    use fvid::playback_native::RawFrame;
    let data = include_bytes!("fixtures/hevc/main-ipb.mp4");
    let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
    let mut expected = Vec::new();
    while reader.read_frame().unwrap() {
        expected.push((reader.frame_interval().unwrap(), reader.rgb().to_vec()));
    }
    for millis in [350, 0, 500, 100] {
        let target = Duration::from_millis(millis);
        let raw = reader.seek_raw(target).unwrap().unwrap();
        assert!(
            !matches!(&raw, RawFrame::Rgb(_)),
            "seek must retain the GPU plane path"
        );
        let interval = reader.frame_interval().unwrap();
        assert!(interval.0 * 1000 <= u128::from(millis) * u128::from(interval.2));
        assert!(interval.1 * 1000 > u128::from(millis) * u128::from(interval.2));
        let reference = expected.iter().find(|(i, _)| *i == interval).unwrap();
        assert_eq!(raw.into_rgb(16 << 20).unwrap(), reference.1);
    }
}
