use fvid::{
    codec::hevc_vps::LayeredVps,
    playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader},
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn read(n: &str) -> Vec<u8> {
    std::fs::read(root().join("hevc-ols-reselection").join(n)).unwrap()
}
#[test]
fn new_vps_reselects_enhancement_ols_in_both_native_frontends() {
    for name in ["eos", "bla", "idr"] {
        let vps = LayeredVps::parse(&read(&format!("{name}.vps")), BUDGET).unwrap();
        let sets = &vps.extension.as_ref().unwrap().output.sets;
        assert_eq!(
            (sets[1].output, sets[2].output, sets[2].necessary),
            (1, 8, 9)
        );
        let mut mkv =
            WebmVideoReader::open_software(Cursor::new(read(&format!("{name}.mkv"))), BUDGET)
                .unwrap();
        let mut mp4 = Mp4VideoReader::open_software(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        for _ in 0..2 {
            let mut count = 0;
            while mkv.read_frame_raw().unwrap().is_some() {
                assert_eq!(
                    mkv.selected_layer_id(),
                    Some(3),
                    "{name} Matroska frame{count}"
                );
                count += 1;
            }
            assert!(count > 2);
            let mut count = 0;
            while mp4.read_frame().unwrap().is_some() {
                assert_eq!(mp4.selected_layer_id(), Some(3), "{name} MP4 frame{count}");
                count += 1;
            }
            assert!(count > 2);
            mkv.rewind();
            mp4.rewind();
        }
    }
}

fn packets(name: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut source = fvid::container::mp4::Mp4Reader::open(
        Cursor::new(read(&format!("{name}.mp4"))),
        Default::default(),
    )
    .unwrap();
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
fn reselection_refusal_keeps_queue_poc_policy_and_retry_intact() {
    use fvid::codec::{
        config::HevcConfig,
        hevc_decoder::{LayeredDecoder, LayeredOutputQueue, SubstreamScan},
    };
    use std::sync::Arc;
    let (cfg, bad) = packets("illegal");
    let (_, good) = packets("eos");
    let config = HevcConfig::parse(&cfg).unwrap();
    let old = Arc::new(
        LayeredVps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 32)
                .unwrap()
                .units[0],
            BUDGET,
        )
        .unwrap(),
    );
    let new = Arc::new(LayeredVps::parse(&read("illegal.vps"), BUDGET).unwrap());
    assert_eq!(new.automatic_output_layer_set().unwrap(), 2);
    let mut scan = SubstreamScan::default();
    for p in &bad {
        scan.observe(p, 4).unwrap();
    }
    let old = scan.select(old, 1, 2).unwrap();
    let new = scan.select(new, 2, 2).unwrap();
    let mut decoder = LayeredDecoder::from_configuration(&cfg, 1, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    for p in &bad[..3] {
        decoder
            .decode_selected_queued_packet(&mut queue, &old, p, 2)
            .unwrap();
    }
    let poc = decoder.poc().clone();
    let index = decoder.next_access_unit();
    let before = queue.clone().drain();
    assert!(
        decoder
            .decode_selected_queued_packet(&mut queue, &new, &bad[3], 2)
            .err()
            .unwrap()
            .to_string()
            .contains("OLS disagrees")
    );
    assert!(
        decoder
            .decode_reselected_queued_packet(&mut queue, &new, &bad[3], 2)
            .err()
            .unwrap()
            .to_string()
            .contains("active VPS changed")
    );
    assert_eq!(decoder.output_layer_set(), 1);
    assert_eq!(decoder.poc(), &poc);
    assert_eq!(decoder.next_access_unit(), index);
    let after = queue.clone().drain();
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(&after) {
        assert!(Arc::ptr_eq(&a.picture, &b.picture));
        assert!(Arc::ptr_eq(&a.binding, &b.binding));
    }
    decoder
        .decode_reselected_queued_packet(&mut queue, &new, &good[3], 2)
        .unwrap();
    assert_eq!(decoder.output_layer_set(), 2);
    decoder.reset_queued(&mut queue);
    assert_eq!(decoder.output_layer_set(), 1);
    assert_eq!(decoder.next_access_unit(), 0);
    assert_eq!(queue.pending_access_units(), 0);
}
#[test]
fn automatic_frontends_preserve_pixels_and_seek_on_each_side_of_ols_change() {
    use fvid::playback_native::RawFrame;
    for name in ["eos", "bla", "idr"] {
        let mut control = Mp4VideoReader::open_software(
            Cursor::new(
                std::fs::read(root().join(format!("hevc-layered-vps-epoch-{name}.mp4"))).unwrap(),
            ),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        let mut expected = Vec::new();
        while let Some(f) = control.read_frame().unwrap() {
            expected.push((
                f.packed.unwrap().frame.data.clone(),
                f.presentation_time.nanoseconds().unwrap(),
                f.sample_index,
            ));
        }
        let mut mkv =
            WebmVideoReader::open_software(Cursor::new(read(&format!("{name}.mkv"))), BUDGET)
                .unwrap();
        let mut mp4 = Mp4VideoReader::open_software(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        for _ in 0..2 {
            for (bytes, pts, _) in &expected {
                let Some(RawFrame::Planar(p)) = mkv.read_frame_raw().unwrap() else {
                    panic!("{name} missing Matroska picture")
                };
                assert_eq!(&p.frame.data, bytes, "{name} Matroska pixels");
                assert_eq!(mkv.frame_interval().unwrap().0, *pts as u128);
                assert_eq!(mkv.selected_layer_id(), Some(3));
                let f = mp4.read_frame().unwrap().unwrap();
                assert_eq!(&f.packed.unwrap().frame.data, bytes, "{name} MP4 pixels");
                assert_eq!(f.presentation_time.nanoseconds().unwrap(), *pts);
                assert_eq!(mp4.selected_layer_id(), Some(3));
            }
            assert!(mkv.read_frame_raw().unwrap().is_none());
            assert!(mp4.read_frame().unwrap().is_none());
            mkv.rewind();
            mp4.rewind();
        }
        for i in [2, 0, 2] {
            let (bytes, pts, index) = &expected[i];
            let Some(RawFrame::Planar(p)) = mkv.seek_to_frame_raw(*pts as i64 + 1).unwrap() else {
                panic!("{name} missing seek picture")
            };
            assert_eq!(&p.frame.data, bytes);
            assert_eq!(mkv.selected_layer_id(), Some(3));
            mp4.seek_to_sync(*index as i64);
            let f = mp4.read_frame().unwrap().unwrap();
            assert_eq!(&f.packed.unwrap().frame.data, bytes, "{name} seek");
            assert_eq!(mp4.selected_layer_id(), Some(3));
        }
    }
}
#[test]
fn illegal_reselection_is_not_hidden_by_automatic_sources() {
    let mut mkv = WebmVideoReader::open_software(Cursor::new(read("illegal.mkv")), BUDGET).unwrap();
    let mut mp4 =
        Mp4VideoReader::open_software(Cursor::new(read("illegal.mp4")), Default::default(), BUDGET)
            .unwrap();
    for _ in 0..2 {
        let error = loop {
            match mkv.read_frame_raw() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("missing Matroska refusal"),
                Err(e) => break e,
            }
        };
        assert!(error.to_string().contains("active VPS changed"), "{error}");
        assert!(
            mkv.read_frame_raw()
                .err()
                .unwrap()
                .to_string()
                .contains("requires rewind")
        );
        mkv.rewind();
        let error = loop {
            match mp4.read_frame() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("missing MP4 refusal"),
                Err(e) => break e,
            }
        };
        assert!(error.to_string().contains("active VPS changed"), "{error}");
        assert!(
            mp4.read_frame()
                .err()
                .unwrap()
                .to_string()
                .contains("requires rewind")
        );
        mp4.rewind();
    }
}
