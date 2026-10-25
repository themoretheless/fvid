use fvid::{
    codec::{config::NalUnits, hevc_decoder::SubstreamScan, hevc_vps::LayeredVps},
    container::mp4::Limits,
    playback::video::mp4::Mp4VideoReader,
    playback_native::NativeReader,
};
use std::{io::Cursor, path::PathBuf, sync::Arc};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-additional-layer-sets")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
#[test]
fn additional_layer_sets_have_conforming_extraction() {
    for family in [
        "pcm",
        "multislice",
        "colour-planes",
        "subset",
        "dependent",
        "external",
        "external-dependent",
    ] {
        for available in [0, 1] {
            let name = format!("{family}-available{available}");
            let input = bytes(&(name.clone() + ".packet"));
            let vps = NalUnits::new(&input, 4).unwrap().next().unwrap().unwrap();
            let vps = Arc::new(LayeredVps::parse(vps, BUDGET).unwrap());
            assert_eq!(
                vps.extension.as_ref().unwrap().output.sets[2].layer_set_index,
                2
            );
            assert_eq!(
                vps.extension.as_ref().unwrap().layer_sets.layer_sets[2],
                if matches!(family, "subset" | "dependent" | "external-dependent") {
                    vec![3, 7]
                } else {
                    vec![3]
                }
            );
            let mut scan = SubstreamScan::default();
            scan.observe(&input, 4).unwrap();
            let selection = scan.select(vps, 2, 0).unwrap();
            assert_eq!(
                selection.layers(),
                if matches!(family, "dependent" | "external-dependent") {
                    (1 << 3) | (1 << 7)
                } else {
                    1 << 3
                }
            );
            assert_eq!(
                selection.extracted_layers(),
                if matches!(family, "subset" | "dependent" | "external-dependent") {
                    (1 << 3) | (1 << 7)
                } else {
                    1 << 3
                }
            );
            let expected = bytes(&(name + ".exported"));
            assert_eq!(
                selection.extract_packet(&input, 4, BUDGET).unwrap(),
                expected
            );
            for width in [1u8, 2] {
                let reframe = |data: &[u8]| -> Vec<u8> {
                    NalUnits::new(data, 4)
                        .unwrap()
                        .map(|n| n.unwrap())
                        .filter(|n| n.len() < (1usize << (8 * width)))
                        .flat_map(|n| {
                            let mut out =
                                (n.len() as u32).to_be_bytes()[4 - width as usize..].to_vec();
                            out.extend_from_slice(n);
                            out
                        })
                        .collect()
                };
                assert_eq!(
                    selection
                        .extract_packet(&reframe(&input), width, BUDGET)
                        .unwrap(),
                    reframe(&expected)
                );
            }
        }
    }
}
#[test]
fn additional_layer_sets_decode_owned_pictures_through_mp4_and_matroska() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let oracle = bytes(c["oracle"].as_str().unwrap());
        let data = bytes(&(name.to_string() + ".mp4"));
        let mut r = Mp4VideoReader::open_software(Cursor::new(&data), Limits::default(), BUDGET)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            for i in 0..2 {
                let f = r
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                    .unwrap_or_else(|| panic!("{name}: missing frame {i}"));
                assert_eq!(f.sample_index, i + c["first"].as_u64().unwrap() as usize);
                assert_eq!(f.presentation_time.ticks, f.sample_index as i64);
                assert_eq!(f.duration.ticks, 1);
                assert_eq!(
                    r.selected_layer_id(),
                    Some(
                        if c["dependent"] == true || c["subset"] == true && c["available"] == true {
                            7
                        } else {
                            3
                        }
                    )
                );
                let p = f.packed.unwrap();
                assert!(
                    p.frame.data == oracle[..p.frame.data.len()],
                    "{name}: first owned frame pixels"
                );
            }
            assert!(r.read_frame().unwrap().is_none());
            r.rewind();
        }
        let mut selected = fvid::playback::video::mp4::LayeredMp4VideoReader::open(
            Cursor::new(&data),
            Limits::default(),
            2,
            0,
            BUDGET,
        )
        .unwrap();
        for _ in 0..2 {
            for _ in 0..2 {
                let f = selected.read_frame().unwrap().unwrap();
                assert_eq!(
                    f.decoded.layer_id,
                    if c["dependent"] == true { 7 } else { 3 }
                );
                if c["external"] == true {
                    assert!(!f.decoded.binding.vps.vps.layers.base_internal);
                    assert!(
                        f.decoded
                            .binding
                            .vps
                            .vps
                            .ordering
                            .iter()
                            .all(|o| o.max_decoded_pictures == 1 && o.max_reorder_pictures == 0)
                    );
                    assert_eq!(
                        f.decoded.binding.sps.output_ordering[0].max_decoded_pictures,
                        6
                    );
                }
                let pixels: Vec<u8> = f
                    .decoded
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap()))
                    .collect();
                assert!(
                    pixels == oracle[..pixels.len()],
                    "{name}: selected OLS2 pixels"
                );
            }
            assert!(selected.read_frame().unwrap().is_none());
            selected.rewind();
        }
        for index in [
            1 + c["first"].as_u64().unwrap() as usize,
            c["first"].as_u64().unwrap() as usize,
        ] {
            assert_eq!(selected.seek_to_sync(index as i64), index as i64);
            let frame = selected.read_frame().unwrap().unwrap();
            assert_eq!(frame.sample_index, index);
            assert_eq!(
                frame.decoded.layer_id,
                if c["dependent"] == true { 7 } else { 3 }
            );
        }
        let mkv = bytes(&(name.to_string() + ".mkv"));
        let mut r = NativeReader::software(Cursor::new(&mkv), BUDGET).unwrap();
        for _ in 0..2 {
            for _ in 0..2 {
                let raw = r
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                    .expect("missing Matroska pixels");
                let fvid::playback_native::RawFrame::Planar(p) = raw else {
                    panic!("{name}: expected owned packed HEVC");
                };
                assert!(
                    p.frame.data == oracle[..p.frame.data.len()],
                    "{name}: Matroska pixels"
                );
                assert_eq!(r.video_codec(), "H.265");
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            r.rewind().unwrap();
        }
        // The public Matroska adapter normalizes to its first displayed
        // timestamp; metadata-only blocks do not extend that visible origin.
        for index in [1u64, 0] {
            assert!(
                r.seek_raw(std::time::Duration::from_millis(index * 40))
                    .unwrap()
                    .is_some()
            );
            let (ticks, scale) = r.current_pts().unwrap();
            assert_eq!(
                u128::try_from(ticks).unwrap() * 25,
                u128::from(index) * u128::from(scale)
            );
        }
    }
}

#[test]
fn additional_selection_errors_publish_no_parameter_poc_or_queue_state() {
    use fvid::{
        codec::hevc_decoder::{LayeredDecoder, LayeredOutputQueue},
        container::mp4::Mp4Reader,
    };
    let data = bytes("pcm-available1-inline0.mp4");
    let demux = Mp4Reader::open_indexed(Cursor::new(&data), Limits::default()).unwrap();
    let config = &demux.tracks()[0].configuration;
    let input = bytes("pcm-available1.packet");
    let bad = bytes("corrupt-root.packet");
    let mut scan = SubstreamScan::default();
    scan.observe(&input, 4).unwrap();
    let nal = NalUnits::new(&input, 4).unwrap().next().unwrap().unwrap();
    let vps = Arc::new(LayeredVps::parse(nal, BUDGET).unwrap());
    let selection = scan.select(vps, 2, 0).unwrap();
    let other = bytes("pcm-available0.packet");
    let nal = NalUnits::new(&other, 4).unwrap().next().unwrap().unwrap();
    let wrong = scan
        .select(Arc::new(LayeredVps::parse(nal, BUDGET).unwrap()), 2, 0)
        .unwrap();
    let mut decoder = LayeredDecoder::from_configuration(config, 2, BUDGET).unwrap();
    let mut queue = LayeredOutputQueue::default();
    let error = decoder
        .decode_selected_queued_packet(&mut queue, &selection, &bad, 0)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "truncated or oversized bit field");
    assert_eq!(decoder.next_access_unit(), 0);
    assert!(decoder.parameters().active(3).is_none());
    assert_eq!(queue.pending_access_units(), 0);
    let error = decoder
        .decode_selected_queued_packet(&mut queue, &wrong, &input, 0)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "HEVC extraction VPS disagrees with active picture"
    );
    assert_eq!(decoder.next_access_unit(), 0);
    assert!(decoder.parameters().active(3).is_none());
    let (ready, end) = decoder
        .decode_selected_queued_packet(&mut queue, &selection, &input, 0)
        .unwrap();
    assert!(end.bitstream);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].layer_id, 3);
    assert_eq!(decoder.next_access_unit(), 1);
    assert!(selection.extract_packet(&input, 4, 16).is_err());
    let mut primed = LayeredDecoder::from_configuration(config, 1, BUDGET).unwrap();
    assert_eq!(
        primed
            .prime_selected_metadata(&wrong, &input)
            .err()
            .unwrap()
            .to_string(),
        "HEVC extraction VPS disagrees with active picture"
    );
    assert!(primed.parameters().active(3).is_none());
    let vps = NalUnits::new(&input, 4).unwrap().next().unwrap().unwrap();
    let original = scan
        .select(Arc::new(LayeredVps::parse(vps, BUDGET).unwrap()), 1, 0)
        .unwrap();
    let (frames, _) = primed
        .decode_selected_queued_packet(&mut LayeredOutputQueue::default(), &original, &input, 0)
        .unwrap();
    assert_eq!(frames.len(), 2);
    let file = bytes("subset-available1-inline0.mp4");
    let demux = Mp4Reader::open_indexed(Cursor::new(&file), Limits::default()).unwrap();
    let packet = bytes("subset-available1.packet");
    let vps = NalUnits::new(&packet, 4).unwrap().next().unwrap().unwrap();
    let mut subset_scan = SubstreamScan::default();
    subset_scan.observe(&packet, 4).unwrap();
    let subset = subset_scan
        .select(Arc::new(LayeredVps::parse(vps, BUDGET).unwrap()), 2, 0)
        .unwrap();
    let mut subset_decoder =
        LayeredDecoder::from_configuration(&demux.tracks()[0].configuration, 2, BUDGET).unwrap();
    let (frames, _) = subset_decoder
        .decode_selected_queued_packet(&mut LayeredOutputQueue::default(), &subset, &packet, 0)
        .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].layer_id, 3);
    assert!(subset_decoder.parameters().active(7).is_none());
    {
        let file = bytes("corrupt-second-root.mp4");
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(file), Limits::default(), BUDGET).unwrap();
        for _ in 0..2 {
            let error = loop {
                match reader.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("missing selected entropy failure"),
                    Err(e) => break e,
                }
            };
            assert_eq!(error.to_string(), "truncated or oversized bit field");
            assert!(
                reader
                    .read_frame()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            reader.rewind();
        }
    }
}
