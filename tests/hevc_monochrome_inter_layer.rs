use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn path(name: &str, extension: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(format!("hevc-layered-mono-inter-{name}.{extension}"))
}
fn source(name: &str) -> LayeredMp4VideoReader<Cursor<Vec<u8>>> {
    LayeredMp4VideoReader::open(
        Cursor::new(std::fs::read(path(name, "mp4")).unwrap()),
        Limits::default(),
        1,
        0,
        BUDGET,
    )
    .unwrap()
}
#[test]
fn monochrome_identity_and_nonidentity_references_match_independent_pixels() {
    for name in ["identity", "region"] {
        let mut source = source(name);
        let expected = std::fs::read(path(name, "yuv")).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut indices = Vec::new();
            while let Some(frame) = source.read_frame().unwrap() {
                indices.push(frame.sample_index);
                assert_eq!(frame.presentation_time.ticks, frame.sample_index as i64);
                assert_eq!(frame.duration.ticks, 1);
                assert_eq!(frame.decoded.layer_id, 3);
                assert_eq!(frame.decoded.binding.sps.sps.chroma_format, 0);
                if name == "region" {
                    assert_eq!(
                        frame.decoded.binding.pps.reference_locations[0].region_offsets,
                        Some([2; 4])
                    );
                }
                assert!(frame.decoded.picture.planes[1].samples().is_empty());
                assert!(frame.decoded.picture.planes[2].samples().is_empty());
                let luma: Vec<u8> = frame.decoded.picture.planes[0]
                    .samples()
                    .iter()
                    .map(|&v| v as u8)
                    .collect();
                let packed = frame.into_video_frame().unwrap().packed.unwrap();
                assert!(packed.frame.data[luma.len()..].iter().all(|&v| v == 128));
                assert!(&packed.frame.data[..luma.len()] == luma.as_slice());
                actual.extend_from_slice(&luma);
            }
            assert_eq!(actual.len(), expected.len(), "packed lengths");
            assert!(
                actual == expected,
                "first mismatch {:?}",
                actual.iter().zip(&expected).position(|(a, b)| a != b)
            );
            assert_eq!(indices, [0, 1]);
            source.rewind();
        }
    }
}

#[test]
fn monochrome_region_fixture_uses_inter_layer_prediction_without_temporal_rps() {
    use fvid::codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::LayeredParameters,
        hevc_nal::NalHeader,
        hevc_slice::SliceHeader,
    };
    use fvid::container::mp4::Mp4Reader;
    let mut movie = Mp4Reader::open(
        Cursor::new(std::fs::read(path("region", "mp4")).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let config = movie.tracks()[0].configuration.clone();
    let config_view = HevcConfig::parse(&config).unwrap();
    let mut packet = Vec::new();
    movie.read_packet(0, 1, &mut packet).unwrap();
    let units: Vec<_> = NalUnits::new(&packet, config_view.length_size)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut parameters = LayeredParameters::from_configuration(&config, BUDGET).unwrap();
    let base = parameters.activate_slice(units[0], 1).unwrap();
    assert_eq!(base.sps.sps.chroma_format, 0);
    let nal = units
        .iter()
        .find(|nal| NalHeader::parse(nal).unwrap().layer_id == 3)
        .unwrap();
    let binding = parameters.activate_slice(nal, 1).unwrap();
    let header = SliceHeader::parse_layered(nal, &binding.context(), BUDGET).unwrap();
    assert!(!header.nal.is_irap());
    assert!(header.short_term.is_empty());
    assert!(header.long_term.is_empty());
    assert_eq!(header.layered.as_ref().unwrap().reference_layers, [0]);
    assert_eq!(
        binding.pps.reference_locations[0].region_offsets,
        Some([2; 4])
    );
}

#[test]
fn monochrome_common_player_and_native_seek_preserve_luma() {
    use fvid::{playback::video::mp4::Mp4VideoReader, playback_native::NativeReader};
    use std::time::Duration;
    for name in ["identity", "region"] {
        let bytes = std::fs::read(path(name, "mp4")).unwrap();
        let expected = std::fs::read(path(name, "yuv")).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes.clone()), Limits::default(), BUDGET)
                .unwrap();
        for _ in 0..2 {
            for index in 0..2 {
                let frame = reader.read_frame().unwrap().unwrap();
                assert_eq!(frame.sample_index, index);
                assert_eq!(reader.selected_layer_id(), Some(3));
                let packed = frame.packed.unwrap();
                assert!(&packed.frame.data[..4096] == &expected[index * 4096..(index + 1) * 4096]);
            }
            assert!(reader.read_frame().unwrap().is_none());
            assert_eq!(reader.seek_to_sync(1), 0);
        }
        let mut native = NativeReader::software(Cursor::new(bytes), BUDGET).unwrap();
        assert!(
            native
                .seek_raw(Duration::from_millis(40))
                .unwrap()
                .is_some()
        );
        native.rewind().unwrap();
        assert!(native.read_frame_raw().unwrap().is_some());
    }
}

#[test]
fn failed_monochrome_resampled_prediction_preserves_queue_and_allows_retry() {
    use fvid::{
        codec::hevc_decoder::{LayeredDecoder, LayeredOutputQueue},
        container::mp4::Mp4Reader,
    };
    use std::sync::Arc;
    let selection_source = source("region");
    let mut good = Mp4Reader::open(
        Cursor::new(std::fs::read(path("region", "mp4")).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let mut bad = Mp4Reader::open(
        Cursor::new(std::fs::read(path("region-truncated", "mp4")).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let config = good.tracks()[0].configuration.clone();
    let mut packet = Vec::new();
    let mut decoder = LayeredDecoder::from_configuration(&config, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    good.read_packet(0, 0, &mut packet).unwrap();
    decoder
        .decode_selected_queued_packet(
            &mut queue,
            selection_source.selection_at(0).unwrap(),
            &packet,
            0,
        )
        .unwrap();
    let poc = decoder.poc().clone();
    let active = decoder.parameters().active(3).unwrap().clone();
    let old = queue.clone().drain();
    bad.read_packet(0, 1, &mut packet).unwrap();
    let error = decoder
        .decode_selected_queued_packet(
            &mut queue,
            selection_source.selection_at(1).unwrap(),
            &packet,
            0,
        )
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("truncated"), "{error}");
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), 1);
    assert!(Arc::ptr_eq(
        decoder.parameters().active(3).unwrap(),
        &active
    ));
    assert_eq!(queue.pending_access_units(), 1);
    for (before, after) in old.iter().zip(queue.clone().drain()) {
        assert!(Arc::ptr_eq(&before.picture, &after.picture));
    }
    good.read_packet(0, 1, &mut packet).unwrap();
    let (mut ready, _) = decoder
        .decode_selected_queued_packet(
            &mut queue,
            selection_source.selection_at(1).unwrap(),
            &packet,
            0,
        )
        .unwrap();
    ready.extend(queue.drain());
    assert_eq!(
        ready
            .iter()
            .map(|p| (p.access_unit, p.layer_id))
            .collect::<Vec<_>>(),
        [(0, 3), (1, 3)]
    );
    let expected = std::fs::read(path("region", "yuv")).unwrap();
    for (index, frame) in ready.iter().enumerate() {
        assert!(
            frame.picture.planes[0]
                .samples()
                .iter()
                .map(|&v| v as u8)
                .eq(expected[index * 4096..(index + 1) * 4096].iter().copied())
        );
    }
}
