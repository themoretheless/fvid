//! Repeated complete target IDR must retain each sub-DPB's prior-output policy.
use fvid::{
    codec::hevc_decoder::{
        LayeredAccessUnit, LayeredDecoder, LayeredOutputQueue, LayeredParameters,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::{
        mp4::{LayeredMp4VideoReader, Mp4VideoReader},
        webm::WebmVideoReader,
    },
    playback_native::RawFrame,
};
use std::{io::Cursor, path::PathBuf, sync::Arc};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join("hevc-idr-output-epochs").join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 12);
    m["cases"].as_array().unwrap().clone()
}
fn expected(c: &serde_json::Value) -> Vec<(usize, u8)> {
    if c["discard"] != true {
        vec![(0, 0), (0, 3), (1, 0), (1, 3), (2, 0), (2, 3)]
    } else if c["independent"] == true {
        vec![(2, 0), (2, 3)]
    } else {
        // The base discard does not discard prior output of a dependent layer.
        vec![(0, 3), (1, 3), (2, 0), (2, 3)]
    }
}
fn oracle(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap()
}
#[test]
fn complete_target_idr_preserves_sub_dpb_policy_pixels_clocks_and_rewind() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(read(&format!("{name}.mp4"))),
            Limits::default(),
            1,
            0,
            BUDGET,
        )
        .unwrap();
        let expected = expected(&c);
        let oracle = oracle(&c);
        for _ in 0..2 {
            let mut ids = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert_eq!(frame.decoded.poc, 0);
                ids.push((frame.sample_index, frame.decoded.layer_id));
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    frame.sample_index as i128 * 40000000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40000000);
                let packed = frame.into_video_frame().unwrap().packed.unwrap();
                assert_eq!(
                    &packed.frame.data,
                    &oracle[..packed.frame.data.len()],
                    "{name}"
                );
            }
            assert_eq!(ids, expected, "{name}");
            reader.rewind();
        }
        for index in [2, 0, 1] {
            reader.seek_to_sync(index);
            let frame = reader.read_frame().unwrap().unwrap();
            let landed = if c["discard"] == true && c["independent"] == true {
                2
            } else {
                index as usize
            };
            assert_eq!(frame.sample_index, landed, "{name} seek");
            let packed = frame.into_video_frame().unwrap().packed.unwrap();
            assert_eq!(
                &packed.frame.data,
                &oracle[..packed.frame.data.len()],
                "{name} seek"
            );
        }
    }
}
#[test]
fn native_frontends_present_retained_prior_enhancement_once_per_access_unit() {
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let mut mp4 = Mp4VideoReader::open(
            Cursor::new(read(&format!("{name}.mp4"))),
            Limits::default(),
            BUDGET,
        )
        .unwrap();
        let mut mkv =
            WebmVideoReader::open(Cursor::new(read(&format!("{name}.mkv"))), BUDGET).unwrap();
        assert!(!mp4.hardware_accelerated());
        assert!(!mkv.hardware_accelerated());
        let expected: Vec<_> = expected(&c)
            .into_iter()
            .filter(|(_, layer)| *layer == 3)
            .map(|(index, _)| index)
            .collect();
        let oracle = oracle(&c);
        for _ in 0..2 {
            for &index in &expected {
                let f = mp4
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} MP4: {e}"))
                    .unwrap();
                assert_eq!(f.sample_index, index, "{name}");
                assert_eq!(mp4.selected_layer_id(), Some(3));
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    index as i128 * 40000000
                );
                let f = f.packed.unwrap();
                assert_eq!(&f.frame.data, &oracle[..f.frame.data.len()]);
                let Some(RawFrame::Planar(f)) = mkv
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{name} Matroska: {e}"))
                else {
                    panic!("{name}: missing frame")
                };
                assert_eq!(mkv.selected_layer_id(), Some(3));
                assert_eq!(
                    mkv.frame_interval().unwrap(),
                    (
                        (index - expected[0]) as u128 * 40_000_000,
                        (index - expected[0] + 1) as u128 * 40_000_000,
                        1_000_000_000,
                    ),
                    "{name} Matroska clock"
                );
                assert_eq!(&f.frame.data, &oracle[..f.frame.data.len()]);
            }
            assert!(mp4.read_frame().unwrap().is_none());
            assert!(mkv.read_frame_raw().unwrap().is_none());
            mp4.rewind();
            mkv.rewind();
        }
        // Establish the display origin before seeking on the relative MKV clock.
        assert!(mkv.read_frame_raw().unwrap().is_some());
        for index in [
            *expected.last().unwrap(),
            expected[0],
            *expected.last().unwrap(),
        ] {
            let relative = (index - expected[0]) as u128 * 40_000_000;
            let Some(RawFrame::Planar(f)) = mkv.seek_to_frame_raw(relative as i64 + 1).unwrap()
            else {
                panic!("{name} seek frame")
            };
            assert_eq!(
                mkv.frame_interval().unwrap().0,
                relative,
                "{name} Matroska seek clock"
            );
            assert_eq!(&f.frame.data, &oracle[..f.frame.data.len()]);
            assert_eq!(mkv.selected_layer_id(), Some(3));
            mp4.seek_to_sync(index as i64);
            let f = mp4.read_frame().unwrap().unwrap();
            assert_eq!(f.sample_index, index, "{name} MP4 seek clock");
            let f = f.packed.unwrap();
            assert_eq!(&f.frame.data, &oracle[..f.frame.data.len()]);
        }
    }
}
fn packets(name: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut source = Mp4Reader::open(Cursor::new(read(name)), Limits::default()).unwrap();
    let cfg = source.tracks()[0].configuration.clone();
    let mut packets = Vec::new();
    for i in 0..source.tracks()[0].samples.len() {
        let mut p = Vec::new();
        source.read_packet(0, i, &mut p).unwrap();
        packets.push(p);
    }
    (cfg, packets)
}
#[test]
fn failed_second_idr_keeps_old_epoch_queue_and_poc_then_valid_retry_succeeds() {
    let (cfg, bad) = packets("pcm-dependent-flush-truncated.mp4");
    let (_, good) = packets("pcm-dependent-flush.mp4");
    let mut decoder = LayeredDecoder::from_configuration(&cfg, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    assert!(
        decoder
            .decode_queued_packet(&mut queue, &good[0], 0)
            .unwrap()
            .0
            .is_empty()
    );
    let before = queue.clone().drain();
    assert_eq!(before.len(), 2);
    let poc = decoder.poc().clone();
    let index = decoder.next_access_unit();
    // Packet/NAL framing and every IDR header are valid: the refusal must
    // occur during enhancement entropy reconstruction, after epoch staging.
    let parameters = LayeredParameters::from_configuration(&cfg, BUDGET).unwrap();
    let plan = LayeredAccessUnit::parse(&parameters, decoder.poc(), &bad[1], 4, 1, BUDGET).unwrap();
    assert_eq!(plan.pictures().len(), 2);
    assert!(plan.pictures().iter().all(|p| p.slices[0].nal.is_idr()));
    let error = decoder
        .decode_queued_packet(&mut queue, &bad[1], 0)
        .err()
        .expect("truncated enhancement entropy must refuse");
    assert_eq!(error.to_string(), "truncated or oversized bit field");
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), index);
    let after = queue.clone().drain();
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(&after) {
        assert!(Arc::ptr_eq(&a.picture, &b.picture));
        assert!(Arc::ptr_eq(&a.binding, &b.binding));
    }
    // Standalone push cannot infer a boundary from an IDR NAL alone. A failed
    // queued decode must not leak its staged epoch and relax this old guard.
    let mut duplicate_poc = before.clone();
    for p in &mut duplicate_poc {
        p.access_unit = index;
    }
    let error = queue
        .push(duplicate_poc, 0, BUDGET)
        .err()
        .expect("old epoch must still reject duplicate POC");
    assert!(error.to_string().contains("epoch boundary"), "{error}");
    let mut out = decoder
        .decode_queued_packet(&mut queue, &good[1], 0)
        .unwrap()
        .0;
    out.extend(
        decoder
            .decode_queued_packet(&mut queue, &good[2], 0)
            .unwrap()
            .0,
    );
    out.extend(queue.drain());
    assert_eq!(
        out.iter()
            .map(|p| (p.access_unit, p.layer_id))
            .collect::<Vec<_>>(),
        [(0, 0), (0, 3), (1, 0), (1, 3), (2, 0), (2, 3)]
    );
    decoder.reset_queued(&mut queue);
    assert_eq!(decoder.next_access_unit(), 0);
    assert_eq!(queue.pending_access_units(), 0);
}
