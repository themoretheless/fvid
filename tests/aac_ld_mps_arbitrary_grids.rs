use fvid_media::owned_aac::aac_ld_mps_decor::Mode;
use fvid_media::owned_aac::{aac_ld_mps_config::Config, aac_sbr_qmf::Complex, bits::BitReader};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join("aac-ld-mps-arbitrary-grids")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn read(c: &Value, key: &str) -> Vec<u8> {
    std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()
}
fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn all_grid_change_headers_and_frames_have_complete_valid_syntax() {
    use fvid_media::owned_aac::{aac_ld_mps_config::ExtensionValue, aac_ld_mps_frame};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 546);
    for c in m["cases"].as_array().unwrap() {
        for i in 0..3 {
            let cfg = Config::parse(&hex(&c["configs"][i])).unwrap();
            cfg.validate_low_delay_profile().unwrap();
            assert_eq!(usize::from(cfg.parameter_bands), n(&c["grids"][i]));
            let ExtensionValue::ArbitraryTree { ott, .. } = &cfg.extensions[0].value else {
                panic!("missing tree")
            };
            assert!(ott.iter().all(|o| o.bands == cfg.parameter_bands));
            let wire = hex(&c["reference"][i]);
            let mut bits = BitReader::new(&wire);
            aac_ld_mps_frame::Decoder::new(&cfg)
                .unwrap()
                .decode(&mut bits)
                .unwrap();
            assert_eq!(bits.remaining(), 0);
        }
    }
}
#[test]
fn grid_change_movies_decode_identical_nonzero_pcm_in_both_containers() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut mp4 = Vec::new();
        let mut mkv = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut mp4)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(c, "file")),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mp4, mkv);
        assert_eq!(mp4.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(mp4.iter().any(|&v| v != 0));
    }
}
#[test]
fn grid_history_matches_original_c_for_every_pair_and_return() {
    use fvid_media::owned_aac::aac_ld_mps_arbitrary_tree::Decoder;
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("grid-components.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 252);
    let input = std::fs::read(root().join("grid-input.f32le")).unwrap();
    let gold = std::fs::read(root().join("grid-expected.f32le")).unwrap();
    let mut at = 0;
    let mut maximum = 0.0f64;
    for c in m["cases"].as_array().unwrap() {
        let nodes: Vec<Vec<bool>> = serde_json::from_value(c["nodes"].clone()).unwrap();
        let positions: Vec<u8> = serde_json::from_value(c["positions"].clone()).unwrap();
        let mut previous: Option<Decoder> = None;
        for f in c["blocks"].as_array().unwrap() {
            let bands = n(&f["bands"]) as u8;
            let cutoffs: Vec<u8> = serde_json::from_value(f["cutoffs"].clone()).unwrap();
            let map: Vec<u8> = serde_json::from_value(f["map"].clone()).unwrap();
            let levels: Vec<Vec<Vec<f64>>> = serde_json::from_value(f["cld"].clone()).unwrap();
            let mut d =
                Decoder::new_for_qmf(&nodes, &cutoffs, &positions, bands, n(&c["bank"])).unwrap();
            if let Some(old) = &previous {
                d.inherit_history(old).unwrap();
            }
            let mut source = vec![vec![vec![Complex::default(); map.len()]; 2]; 3];
            let mut offset = n(&f["source_offset"]);
            for row in &mut source {
                for channel in row {
                    for z in channel {
                        z.re = f32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
                            as f64;
                        z.im = f32::from_le_bytes(input[offset + 4..offset + 8].try_into().unwrap())
                            as f64;
                        offset += 8;
                    }
                }
            }
            let out = d
                .process(&source, &[0, 2], &levels, &map, &[1., 1.])
                .unwrap();
            for row in out {
                for channel in row {
                    for z in channel {
                        for value in [z.re, z.im] {
                            let want =
                                f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()) as f64;
                            at += 4;
                            let error = (value - want).abs();
                            maximum = maximum.max(error);
                            assert!(error < 2e-8, "{} error {error}", c["name"]);
                        }
                    }
                }
            }
            previous = Some(d);
        }
        let mut last = previous.unwrap();
        last.reset();
        let bands = n(&c["blocks"][2]["bands"]) as u8;
        let fresh = Decoder::new_for_qmf(
            &nodes,
            &vec![bands; n(&c["boxes"])],
            &positions,
            bands,
            n(&c["bank"]),
        )
        .unwrap();
        assert_eq!(last, fresh, "active and dormant history reset");
    }
    assert_eq!(at, gold.len());
    println!(
        "Original C grid changes: {} scalars, max error {maximum}",
        at / 4
    );
}
#[test]
fn changed_paths_and_partial_grid_conversion_still_refuse_atomically() {
    use fvid_media::owned_aac::aac_ld_mps_arbitrary_tree::Decoder;
    let old = Decoder::new(
        &[vec![true, false, false], vec![false]],
        &[23],
        &[0, 1, 2],
        23,
    )
    .unwrap();
    let mut changed = Decoder::new(
        &[vec![false], vec![true, false, false]],
        &[4],
        &[0, 1, 2],
        4,
    )
    .unwrap();
    let saved = changed.clone();
    assert!(changed.inherit_history(&old).is_err());
    assert_eq!(changed, saved);
    let partial = Decoder::new_for_qmf(
        &[vec![true, false, false], vec![false]],
        &[20],
        &[0, 1, 2],
        23,
        32,
    )
    .unwrap();
    let mut full = Decoder::new_for_qmf(
        &[vec![true, false, false], vec![false]],
        &[12],
        &[0, 1, 2],
        12,
        32,
    )
    .unwrap();
    let saved = full.clone();
    assert!(full.inherit_history(&partial).is_err());
    assert_eq!(full, saved);
}
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&packet);
        if let Some(f) = reader
            .decode_source_packet(decoder.as_mut(), &packet, stamp)
            .unwrap()
        {
            if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn downscaled_spatial_playback_seek_and_rewind_keep_source_windows() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(c, "file")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(c, "video")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(play(reader.as_mut()), full);
            let core = n(&c["out_n"]);
            let spatial = core * n(&c["group"]);
            for sample in [
                1,
                core - 1,
                core,
                core + 1,
                spatial - 1,
                spatial,
                spatial + 1,
                n(&c["samples"]) / 2,
                n(&c["samples"]),
            ] {
                reader.rewind();
                let ticks = (sample as u128 * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * n(&c["channels"])
                    * 4;
                assert_eq!(
                    play(reader.as_mut()),
                    full[begin.min(full.len())..],
                    "{} seek{sample} mkv{mkv}",
                    c["name"]
                );
            }
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full);
        }
    }
}

#[test]
fn changing_grids_preserve_packet_clocks_and_checkpoint_retry_in_both_modes() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        for mode in [Mode::Real, Mode::Complex] {
            let mut d = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(usize::from(d.channels()), n(&c["channels"]));
            assert_eq!(d.channel_mask(), 0);
            let mut out = Vec::new();
            for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = n(&r["offset"]);
                let packet = &blob[at..at + n(&r["bytes"])];
                let pts = (i * n(&c["out_n"])) as i64;
                let duration = n(&c["out_n"]) as u64;
                let state = d.checkpoint();
                let bytes = d.retained_payload_bytes().unwrap();
                assert!(
                    d.decode_timed(&packet[..packet.len() - 1], pts, duration)
                        .is_err()
                );
                assert_eq!(d.retained_payload_bytes().unwrap(), bytes);
                let frame = d.decode_timed(packet, pts, duration).unwrap();
                d.restore(&state).unwrap();
                assert_eq!(d.decode_timed(packet, pts, duration).unwrap(), frame);
                if let Some(f) = frame {
                    assert_eq!(f.pts, pts);
                    assert_eq!(f.duration, duration);
                    out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
                }
            }
            while let Some(f) = d.finish().unwrap() {
                out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            assert_eq!(out.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
            assert!(out.iter().any(|&v| v != 0));
        }
    }
}
