use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pixels(p: &IntraPicture) -> Vec<u8> {
    let mut out = Vec::new();
    p.write_planar(&mut out).unwrap();
    out
}
#[test]
fn data_partitions_reconstruct_owned_pcm_and_nonzero_inter_pixels() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 14);
    for case in m["cases"].as_array().unwrap() {
        let mut decoder = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
        let mut control = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            let seed = read("seed.packet");
            assert_eq!(
                pixels(&decoder.decode_order(&seed).unwrap().unwrap()),
                oracle[..768]
            );
            control.decode_order(&seed).unwrap().unwrap();
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let picture = decoder
                    .decode_order(&read(frame["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} frame{}: {e}", case["name"], i + 1))
                    .unwrap();
                let expected = control
                    .decode_order(&read(frame["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    pixels(&picture),
                    pixels(&expected),
                    "{} frame{}",
                    case["name"],
                    i + 1
                );
                assert_eq!(
                    pixels(&picture),
                    oracle[(i + 1) * 768..(i + 2) * 768],
                    "{} frame{} scalar",
                    case["name"],
                    i + 1
                );
            }
            decoder.reset();
            control.reset();
        }
    }
}
#[test]
fn aso_partitions_associate_by_slice_id_across_interleaved_nals() {
    let mut decoder = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
    let mut control = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
    for d in [&mut decoder, &mut control] {
        d.decode_order(&read("seed.packet")).unwrap();
    }
    let actual = decoder
        .decode_order(&read("interleaved.packet"))
        .unwrap()
        .unwrap();
    let expected = control
        .decode_order(&read("interleaved-control.packet"))
        .unwrap()
        .unwrap();
    assert_eq!(pixels(&actual), pixels(&expected));
}
#[test]
fn invalid_data_partitions_fail_and_reset_restores_decoding() {
    for case in manifest()["invalid"].as_array().unwrap() {
        let mut decoder = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
        decoder.decode_order(&read("seed.packet")).unwrap();
        let err = decoder
            .decode_order(&read(case["file"].as_str().unwrap()))
            .unwrap_err()
            .to_string();
        assert!(
            err.to_lowercase()
                .contains(&case["message"].as_str().unwrap().to_lowercase()),
            "{}: {err}",
            case["file"]
        );
        assert!(
            decoder
                .decode_order(&read("P-inter-pcm-split0-1.packet"))
                .is_err()
        );
        decoder.reset();
        decoder.decode_order(&read("seed.packet")).unwrap();
        decoder
            .decode_order(&read("P-inter-pcm-split0-1.packet"))
            .unwrap()
            .unwrap();
    }
}
#[cfg(feature = "player")]
#[test]
fn software_mp4_data_partitions_preserve_frames_through_rewind_and_seek() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for case in manifest()["cases"].as_array().unwrap() {
        let mut reader = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(case["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for pass in 0..3 {
            for i in 0..3 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                assert_eq!(pixels(&frame.picture), oracle[i * 768..(i + 1) * 768]);
            }
            assert!(reader.read_frame().unwrap().is_none());
            if pass == 0 {
                reader.rewind();
            } else {
                assert_eq!(reader.seek_to_sync(2), 0);
            }
        }
    }
}

#[test]
fn partition_preparation_preserves_slice_ranges_and_exact_packet_byte_bound() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        config::AvcConfig,
    };
    let configuration = read("configuration.avcc");
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    for case in manifest()["cases"].as_array().unwrap() {
        for frame in case["frames"].as_array().unwrap() {
            let packet = read(frame["packet"].as_str().unwrap());
            let control = read(frame["control"].as_str().unwrap());
            let actual = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            let expected = prepare(&control, 4, &sps, &pps, control.len()).unwrap();
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(&expected) {
                assert_eq!(a.macroblocks, b.macroblocks);
                assert_eq!(a.header.first_mb, b.header.first_mb);
                assert_eq!(a.header.slice_qp, b.header.slice_qp);
                assert!(a.header.data_partitions.is_some());
                assert_eq!(a.nal[0] & 31, 2);
            }
            assert!(
                prepare(&packet, 4, &sps, &pps, packet.len() - 1)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("byte limit")
            );
        }
    }
}

#[test]
fn mixed_pps_redundant_counter_presence_selects_primary_partition_headers() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare_with_parameters,
        config::AvcConfig,
    };
    let configuration = read("mixed-counter.avcc");
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let parameters: Vec<_> = config
        .pps
        .iter()
        .map(|nal| (sps.clone(), Pps::parse(nal, &sps).unwrap()))
        .collect();
    assert!(!parameters[0].1.redundant_pic_cnt_present);
    assert!(parameters[1].1.redundant_pic_cnt_present);
    let packet = read("mixed-counter.packet");
    let selected = prepare_with_parameters(
        &packet,
        4,
        &sps,
        &parameters[0].1,
        &parameters,
        packet.len(),
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].header.redundant_pic_cnt, 0);
    assert_eq!(
        selected[0]
            .header
            .data_partitions
            .as_ref()
            .unwrap()
            .intra
            .as_ref()
            .unwrap()
            .entropy_bit_offset,
        1
    );
}

#[test]
fn mixed_counter_pictures_keep_primary_pixels_through_reset_and_controls() {
    let m = manifest();
    assert_eq!(m["mixed_counter"].as_array().unwrap().len(), 4);
    let configuration = read("mixed-counter.avcc");
    for case in m["mixed_counter"].as_array().unwrap() {
        let mut decoder = AvcDecoder::new(&configuration, 1 << 20).unwrap();
        let mut control = AvcDecoder::new(&configuration, 1 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for d in [&mut decoder, &mut control] {
                assert_eq!(
                    pixels(
                        &d.decode_order(&read("mixed-counter-seed.packet"))
                            .unwrap()
                            .unwrap()
                    ),
                    oracle[..768]
                );
            }
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let picture = decoder
                    .decode_order(&read(frame["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(frame["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                assert_eq!(pixels(&picture), pixels(&expected), "{}", case["name"]);
                assert_eq!(
                    pixels(&picture),
                    oracle[(i + 1) * 768..(i + 2) * 768],
                    "{}",
                    case["name"]
                );
            }
            decoder.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn mixed_counter_mp4_rewind_and_seek_do_not_emit_redundant_pictures() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for case in manifest()["mixed_counter"].as_array().unwrap() {
        let mut r = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(case["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for pass in 0..3 {
            for i in 0..3 {
                let f = r
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                assert_eq!(pixels(&f.picture), oracle[i * 768..(i + 1) * 768]);
            }
            assert!(r.read_frame().unwrap().is_none());
            if pass == 0 {
                r.rewind();
            } else {
                assert_eq!(r.seek_to_sync(2), 0);
            }
        }
    }
}
#[test]
fn partition_picture_order_rejects_reintroduced_primary_and_descending_redundancy() {
    for (name, message) in [
        ("late-primary-a.packet", "out of redundant counter order"),
        ("descending-rank.packet", "out of redundant counter order"),
        ("late-primary-b.packet", "redundant_pic_cnt outside range"),
    ] {
        let mut d = AvcDecoder::new(&read("mixed-counter.avcc"), 1 << 20).unwrap();
        d.decode_order(&read("mixed-counter-seed.packet")).unwrap();
        let e = d.decode_order(&read(name)).unwrap_err().to_string();
        assert!(e.contains(message), "{name}: {e}");
        assert!(
            d.decode_order(&read("mixed-counter-split0-ranks1-1.packet"))
                .is_err()
        );
        d.reset();
        d.decode_order(&read("mixed-counter-seed.packet")).unwrap();
        d.decode_order(&read("mixed-counter-split0-ranks1-1.packet"))
            .unwrap()
            .unwrap();
    }
}

#[test]
fn redundant_partition_payloads_have_valid_pcm_syntax_and_distinct_samples() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_slice::{SliceHeader, SliceType},
        bits::{BitReader, unescape_rbsp},
        config::{AvcConfig, NalUnits},
    };
    let configuration = read("mixed-counter.avcc");
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps: Vec<_> = config
        .pps
        .iter()
        .map(|p| Pps::parse(p, &sps).unwrap())
        .collect();
    let mut checked = 0;
    for case in manifest()["mixed_counter"].as_array().unwrap() {
        for frame in case["frames"].as_array().unwrap() {
            let packet = read(frame["packet"].as_str().unwrap());
            let mut owner = None;
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let nal = nal.unwrap();
                if nal[0] & 31 == 2 {
                    let id = SliceHeader::parameter_set_id(nal).unwrap();
                    owner = Some(
                        SliceHeader::parse(nal, &sps, pps.iter().find(|p| p.id == id).unwrap())
                            .unwrap(),
                    );
                } else if nal[0] & 31 == 3 {
                    let owner = owner.as_ref().unwrap();
                    if owner.redundant_pic_cnt == 0 {
                        continue;
                    }
                    checked += 1;
                    assert_eq!(owner.slice_type, SliceType::I);
                    let rbsp = unescape_rbsp(&nal[1..]).unwrap();
                    let mut samples = BitReader::new(&rbsp);
                    assert_eq!(
                        samples.unsigned_golomb().unwrap(),
                        owner.data_partitions.as_ref().unwrap().slice_id
                    );
                    assert_eq!(samples.unsigned_golomb().unwrap(), owner.redundant_pic_cnt);
                    let mut control = BitReader::new(&owner.rbsp);
                    control.skip(owner.entropy_bit_offset).unwrap();
                    let mut mb = owner.first_mb as usize;
                    while control.more_rbsp_data() {
                        assert!(mb < 2);
                        assert_eq!(control.unsigned_golomb().unwrap(), 25);
                        while samples.position() % 8 != 0 {
                            assert_eq!(samples.read(1).unwrap(), 0);
                        }
                        for (plane, (bw, bh)) in [(16, 16), (8, 8), (8, 8)].into_iter().enumerate()
                        {
                            for y in 0..bh {
                                for x in 0..bw {
                                    let expected =
                                        2 + ((mb * bw + x) * 11 + y * 17 + plane * 41) % 250;
                                    assert_eq!(samples.read(8).unwrap(), expected as u32);
                                }
                            }
                        }
                        mb += 1;
                    }
                    let count = if case["name"].as_str().unwrap().contains("split1") {
                        1
                    } else {
                        2
                    };
                    assert_eq!(mb, owner.first_mb as usize + count);
                    control.finish_rbsp().unwrap();
                    samples.finish_rbsp().unwrap();
                }
            }
        }
    }
    assert_eq!(checked, 24);
}

#[cfg(feature = "player")]
#[test]
fn software_mp4_refuses_bad_partition_picture_order_after_valid_seed() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for (name, message) in [
        ("late-primary-a.mp4", "out of redundant counter order"),
        ("descending-rank.mp4", "out of redundant counter order"),
        ("late-primary-b.mp4", "redundant_pic_cnt outside range"),
    ] {
        let mut reader = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(name)),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            // Reorder buffering may discover the bad picture before returning
            // the valid seed. Any returned picture must still be that seed.
            let mut returned = 0;
            let error = loop {
                match reader.read_frame() {
                    Err(e) => break e.to_string(),
                    Ok(Some(frame)) => {
                        assert_eq!(returned, 0, "{name}: invalid picture emitted");
                        assert_eq!(
                            pixels(&frame.picture),
                            read("mixed-counter-split0-ranks1.yuv")[..768]
                        );
                        returned += 1;
                    }
                    Ok(None) => panic!("{name}: invalid coded-picture order accepted"),
                }
            };
            assert!(error.contains(message), "{name}: {error}");
            if pass == 0 {
                reader.rewind();
            }
        }
    }
}
