use fvid_media::owned_aac::{
    NativeAacDecoder,
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_pcm,
    aac_ld_mps_session::{Session, Stamp},
    aac_ld_mps_transport::{Extension, Kind, Message},
    bits::BitReader,
};
fn file(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aac-eld-mps-core")
            .join(name),
    )
    .unwrap()
}
fn setup(tree: usize) -> (Config, Vec<Vec<Vec<f64>>>, Vec<Message>) {
    let config = Config::parse(&file(&format!("tree{tree}.ssc"))).unwrap();
    let channels = usize::from(config.tree.input_channels);
    let mut decoder = NativeAacDecoder::new(&file(&format!("tree{tree}.asc"))).unwrap();
    let mut pcm = Vec::new();
    let mut messages = Vec::new();
    for i in 0..3 {
        let samples = decoder
            .decode(&file(&format!("tree{tree}-{i}.au")))
            .unwrap();
        pcm.push(
            (0..channels)
                .map(|c| {
                    samples
                        .chunks_exact(channels)
                        .map(|r| f64::from(r[c]))
                        .collect()
                })
                .collect(),
        );
        messages.push(Message {
            extension: Extension::LowDelay,
            kind: Kind::Frame,
            payload: file(&format!("tree{tree}-{i}.spatial")),
        });
    }
    (config, pcm, messages)
}
fn slice(pcm: &[Vec<f64>], a: usize, b: usize) -> Vec<Vec<f64>> {
    pcm.iter().map(|c| c[a..b].to_vec()).collect()
}
#[test]
fn timed_real_eld_core_is_chunk_invariant_checkpointable_and_resettable() {
    for tree in 0..8 {
        let (config, pcm, messages) = setup(tree);
        let fresh = Session::new(&config, Mode::Complex).unwrap();
        let mut session = fresh.clone();
        let mut direct = aac_ld_mps_pcm::Decoder::new(&config, Mode::Complex).unwrap();
        for i in 0..3 {
            let mut reference_bits = BitReader::new(&messages[i].payload);
            let residuals =
                vec![None; config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes)];
            let expected = direct
                .decode(&mut reference_bits, &pcm[i], &residuals)
                .unwrap();
            // Fractional endpoints in a 1 kHz clock: 10, 11, 11 ticks; negative PTS.
            let start = -11 + (i as i64 * 512 * 1000 / 48000);
            let end = -11 + ((i as i64 + 1) * 512 * 1000 / 48000);
            let duration = (end - start) as u64;
            let a = slice(&pcm[i], 0, 171);
            let b = slice(&pcm[i], 171, 512);
            let split = duration * 171 / 512;
            let stamp = Stamp {
                pts: start,
                duration: split,
                timescale: 1000,
            };
            assert!(
                session
                    .submit(Some((&a, stamp)), &messages[i..i + 1])
                    .unwrap()
                    .is_empty()
            );
            let saved = session.clone();
            assert!(session.finish().is_err());
            assert_eq!(session, saved);
            let stamp = Stamp {
                pts: start + split as i64,
                duration: duration - split,
                timescale: 1000,
            };
            let result = session.submit(Some((&b, stamp)), &[]).unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].pcm, expected);
            assert_eq!(
                result[0].stamp,
                Stamp {
                    pts: start,
                    duration,
                    timescale: 1000
                }
            );
            let mut replay = saved;
            assert_eq!(replay.submit(Some((&b, stamp)), &[]).unwrap(), result);
            assert_eq!(replay, session);
            session.finish().unwrap();
        }
        session.reset();
        assert_eq!(session, fresh);
    }
}
#[test]
fn packet_splitting_and_late_parameters_conserve_every_sample_and_tick() {
    let (config, pcm, messages) = setup(7);
    let mut session = Session::new(&config, Mode::Complex).unwrap();
    let mut joined = vec![Vec::new(); pcm[0].len()];
    for frame in &pcm {
        for (dst, c) in joined.iter_mut().zip(frame) {
            dst.extend(c);
        }
    }
    assert!(
        session
            .submit(
                Some((
                    &joined,
                    Stamp {
                        pts: -2,
                        duration: 1,
                        timescale: 90000
                    }
                )),
                &[]
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(session.queued_samples(), 1536);
    let saved = session.clone();
    let result = session.submit(None, &messages).unwrap();
    assert_eq!(
        result.iter().map(|r| r.stamp.duration).collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert!(
        result
            .iter()
            .all(|r| r.stamp.pts == -2 && r.stamp.timescale == 90000)
    );
    assert_eq!(
        result
            .iter()
            .map(|r| r.pcm.samples.len() / r.pcm.channels)
            .sum::<usize>(),
        1536
    );
    assert_eq!(session.queued_samples(), 0);
    assert_eq!(session.queued_frames(), 0);
    session.finish().unwrap();
    let mut replay = saved;
    assert_eq!(replay.submit(None, &messages).unwrap(), result);
    assert_eq!(replay, session);
}
#[test]
fn malformed_late_spatial_frame_rolls_back_earlier_outputs_and_queued_pcm() {
    let (config, pcm, messages) = setup(7);
    let mut session = Session::new(&config, Mode::Complex).unwrap();
    for (i, frame) in pcm.iter().enumerate() {
        session
            .submit(
                Some((
                    frame,
                    Stamp {
                        pts: i as i64 * 512,
                        duration: 512,
                        timescale: 48000,
                    },
                )),
                &[],
            )
            .unwrap();
    }
    let saved = session.clone();
    let mut invalid = messages.clone();
    invalid[1].payload.push(0xa5);
    let error = session.submit(None, &invalid).unwrap_err().to_string();
    assert!(error.contains("trailing bytes"), "{error}");
    assert_eq!(session, saved);
    assert_eq!(session.submit(None, &messages).unwrap().len(), 3);
    session.finish().unwrap();
}
#[test]
fn incompatible_clock_spans_and_bad_core_are_atomic() {
    let (config, pcm, messages) = setup(7);
    for (stamp, reason) in [
        (
            Stamp {
                pts: 256,
                duration: 256,
                timescale: 90000,
            },
            "different source clocks",
        ),
        (
            Stamp {
                pts: 257,
                duration: 256,
                timescale: 48000,
            },
            "discontinuous",
        ),
        (
            Stamp {
                pts: 256,
                duration: 0,
                timescale: 48000,
            },
            "timed and untimed",
        ),
    ] {
        let mut session = Session::new(&config, Mode::Complex).unwrap();
        session
            .submit(
                Some((
                    &slice(&pcm[0], 0, 256),
                    Stamp {
                        pts: 0,
                        duration: 256,
                        timescale: 48000,
                    },
                )),
                &messages[..1],
            )
            .unwrap();
        let saved = session.clone();
        let error = session
            .submit(Some((&slice(&pcm[0], 256, 512), stamp)), &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains(reason), "{error}");
        assert_eq!(session, saved);
    }
    let mut session = Session::new(&config, Mode::Complex).unwrap();
    let saved = session.clone();
    for stamp in [
        Stamp {
            pts: i64::MAX,
            duration: 1,
            timescale: 48000,
        },
        Stamp {
            pts: 0,
            duration: 1,
            timescale: 0,
        },
    ] {
        assert!(
            session
                .submit(Some((&pcm[0], stamp)), &messages[..1])
                .is_err()
        );
        assert_eq!(session, saved);
    }
    let mut bad = pcm[0].clone();
    bad[0][100] = f64::NAN;
    assert!(
        session
            .submit(
                Some((
                    &bad,
                    Stamp {
                        pts: 0,
                        duration: 512,
                        timescale: 48000
                    }
                )),
                &[]
            )
            .is_err()
    );
    assert_eq!(session, saved);
    let mut header = messages[0].clone();
    header.kind = Kind::HeaderAndFrame;
    assert!(
        session
            .submit(None, &[header])
            .unwrap_err()
            .to_string()
            .contains("in-band")
    );
    assert_eq!(session, saved);
}
#[test]
fn untimed_chunks_keep_first_stamp_even_when_later_pts_decrease() {
    let (config, pcm, messages) = setup(7);
    let mut session = Session::new(&config, Mode::Complex).unwrap();
    session
        .submit(
            Some((
                &slice(&pcm[0], 0, 256),
                Stamp {
                    pts: 9,
                    duration: 0,
                    timescale: 48000,
                },
            )),
            &[],
        )
        .unwrap();
    let out = session
        .submit(
            Some((
                &slice(&pcm[0], 256, 512),
                Stamp {
                    pts: -8,
                    duration: 0,
                    timescale: 48000,
                },
            )),
            &messages[..1],
        )
        .unwrap();
    assert_eq!(
        out[0].stamp,
        Stamp {
            pts: 9,
            duration: 0,
            timescale: 48000
        }
    );
    session.finish().unwrap();
}

#[test]
fn actual_access_units_keep_pcm_and_pts_when_parameters_arrive_late_or_fragmented() {
    use fvid_media::owned_aac::aac_ld_mps_transport::Assembler;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-session");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root.join("packets.bin")).unwrap();
    let mut references = std::collections::BTreeMap::new();
    for case in manifest["cases"].as_array().unwrap() {
        let tree = case["tree"].as_u64().unwrap() as usize;
        let config = Config::parse(&file(&format!("tree{tree}.ssc"))).unwrap();
        let mut session = Session::new(&config, Mode::Complex).unwrap();
        let mut assembler = Assembler::default();
        let mut core = NativeAacDecoder::new(&file(&format!("tree{tree}.asc"))).unwrap();
        let mut output = Vec::new();
        let mode = case["mode"].as_str().unwrap();
        for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = frame["offset"].as_u64().unwrap() as usize;
            let n = frame["bytes"].as_u64().unwrap() as usize;
            let packet = &packets[at..at + n];
            let samples = core.decode(packet).unwrap();
            let channels = usize::from(config.tree.input_channels);
            let planar: Vec<Vec<f64>> = (0..channels)
                .map(|c| {
                    samples
                        .chunks_exact(channels)
                        .map(|r| f64::from(r[c]))
                        .collect()
                })
                .collect();
            let mut bits = BitReader::new(packet);
            bits.skip(frame["core_bits"].as_u64().unwrap() as usize)
                .unwrap();
            let messages = fvid_media::owned_aac::aac_pce::read_eld_spatial_extensions(
                &mut bits,
                &mut assembler,
            )
            .unwrap();
            assert_eq!(bits.remaining(), 0);
            let stamp = Stamp {
                pts: frame["pts"].as_i64().unwrap(),
                duration: frame["duration"].as_u64().unwrap(),
                timescale: 48000,
            };
            let checkpoint = session.clone();
            if mode == "malformed-late" && index == 2 {
                let error = session
                    .submit(Some((&planar, stamp)), &messages)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("trailing bytes"), "{error}");
                assert_eq!(session, checkpoint);
                assert_eq!(session.queued_samples(), 1024);
                break;
            }
            let rendered = session.submit(Some((&planar, stamp)), &messages).unwrap();
            let mut replay = checkpoint;
            assert_eq!(
                replay.submit(Some((&planar, stamp)), &messages).unwrap(),
                rendered
            );
            assert_eq!(replay, session);
            if mode != "immediate" && index < 2 {
                assert!(rendered.is_empty());
            }
            output.extend(rendered);
        }
        assembler.finish().unwrap();
        if mode == "malformed-late" {
            continue;
        }
        if mode == "unfinished" {
            assert!(
                session
                    .finish()
                    .unwrap_err()
                    .to_string()
                    .contains("unfinished")
            );
            assert_eq!(session.queued_samples(), 1536);
            continue;
        }
        session.finish().unwrap();
        assert_eq!(output.len(), 3);
        for (i, frame) in output.iter().enumerate() {
            assert_eq!(
                frame.stamp,
                Stamp {
                    pts: i as i64 * 512,
                    duration: 512,
                    timescale: 48000
                }
            );
        }
        if mode == "immediate" {
            references.insert(tree, output);
        } else {
            assert_eq!(&output, &references[&tree], "{}", case["name"]);
        }
    }
}

#[test]
fn scheduling_movies_still_reproduce_specific_native_spatial_refusal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-session");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let movie =
            std::fs::read(root.join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("LD-MPS spatial reconstruction"),
            "{}: {error}",
            case["name"]
        );
        assert!(pcm.is_empty());
    }
}
#[test]
#[ignore = "pending native spatial dispatch and retained-memory integration"]
fn native_movies_match_immediate_pcm_with_late_and_fragmented_parameters() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-session");
    for tree in 0..8 {
        let mut reference = Vec::new();
        for mode in ["immediate", "late", "fragments"] {
            let movie = std::fs::read(root.join(format!("tree{tree}-{mode}.mp4"))).unwrap();
            let mut pcm = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
            assert!(!pcm.is_empty());
            if mode == "immediate" {
                reference = pcm;
            } else {
                assert_eq!(pcm, reference);
            }
        }
    }
}

#[test]
fn fragment_storage_and_snapshots_are_counted_separately_from_spatial_session() {
    use fvid_media::owned_aac::aac_ld_mps_transport::Assembler;
    let mut assembler = Assembler::default();
    assert_eq!(assembler.retained_payload_bytes().unwrap(), 0);
    assembler
        .read(&mut BitReader::new(&[0x92, 1, 2, 3, 4, 5]), 48)
        .unwrap();
    let before = assembler.retained_payload_bytes().unwrap();
    assert!(before >= 5);
    let saved = assembler.clone();
    assert_eq!(
        assembler
            .retained_payload_bytes_with_checkpoint(Some(&saved))
            .unwrap(),
        before + saved.retained_payload_bytes().unwrap()
    );
    assert_eq!(
        assembler
            .retained_payload_bytes_with_checkpoint(Some(&assembler))
            .unwrap(),
        before
    );
    assembler.reset();
    assert_eq!(assembler.retained_payload_bytes().unwrap(), 0);
    assert!(saved.retained_payload_bytes().unwrap() >= 5);
}
