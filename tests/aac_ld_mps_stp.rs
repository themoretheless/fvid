use fvid_media::owned_aac::{aac_ld_mps_stp::Decoder, aac_sbr_qmf::Complex};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-stp")
}
fn manifest(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn targets(v: &Value) -> Option<Vec<bool>> {
    v["targets"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_bool().unwrap()).collect())
}
#[test]
fn native_stp_movies_accept_active_shaping_instead_of_refusing_pcm() {
    let m = manifest("manifest.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 400);
    for c in m["cases"].as_array().unwrap() {
        let movie =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let cfg = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&hex(c["asc"]
            .as_str()
            .unwrap()))
        .unwrap();
        assert_eq!(
            pcm.len(),
            n(&c["samples"]) * usize::from(cfg.output_channels()) * 4
        );
        assert!(pcm.iter().any(|&v| v != 0));
    }
}
fn reference<const B: usize>(c: &Value, golden: &[u8]) -> Vec<f64> {
    use fvid_media::owned_aac::{
        NativeAacDecoder,
        aac_ld_mps_config::Config,
        aac_ld_mps_decor::Mode,
        aac_ld_mps_pcm::native_order,
        aac_ld_mps_qmf,
        aac_ld_qmf::{Analysis, Synthesis},
        bits::BitReader,
    };
    let tree = n(&c["tree"]);
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    let cfg = Config::parse(&hex(c["ordinary_ssc"].as_str().unwrap())).unwrap();
    assert_eq!(cfg.temporal_shape, 0);
    assert_eq!(cfg.downmix_gain, 0);
    let outputs = usize::from(cfg.tree.output_channels);
    let mut qmf = aac_ld_mps_qmf::Decoder::new(&cfg, Mode::Complex).unwrap();
    let mut stp: Option<Decoder> = None;
    let mut analysis: Vec<_> = (0..inputs).map(|_| Analysis::<B>::new().unwrap()).collect();
    let mut synthesis: Vec<_> = (0..outputs)
        .map(|_| {
            if B == 32 && !c["qmf"].is_null() {
                Synthesis::<B>::new_single_rate_ld_sbr().unwrap()
            } else {
                Synthesis::<B>::new().unwrap()
            }
        })
        .collect();
    let parent = root().parent().unwrap().to_path_buf();
    let mut core = NativeAacDecoder::new(
        &std::fs::read(parent.join(format!("aac-eld-mps-core/tree{tree}.asc"))).unwrap(),
    )
    .unwrap();
    let mut result = Vec::new();
    for (i, frame) in c["frames"].as_array().unwrap().iter().enumerate() {
        let mut input = vec![vec![vec![Complex::default(); B]; inputs]; slots];
        if c["qmf"].is_null() {
            let decoded = core
                .decode(
                    &std::fs::read(
                        parent.join(format!("aac-eld-mps-core/tree{tree}-{}.au", i % 3)),
                    )
                    .unwrap(),
                )
                .unwrap();
            for ch in 0..inputs {
                let pcm: Vec<_> = decoded
                    .chunks_exact(inputs)
                    .map(|s| f64::from(s[ch]))
                    .collect();
                for (t, row) in analysis[ch].process(&pcm).unwrap().into_iter().enumerate() {
                    input[t][ch] = row.to_vec();
                }
            }
        } else {
            let offset = n(&c["qmf"]["expected_offset"]);
            let stored = n(&c["qmf"]["expected_bands"]);
            for t in 0..slots {
                for ch in 0..inputs {
                    for b in 0..B.min(stored) {
                        let at = offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                        input[t][ch][b] = Complex {
                            re: f64::from_le_bytes(golden[at..at + 8].try_into().unwrap()),
                            im: f64::from_le_bytes(golden[at + 8..at + 16].try_into().unwrap()),
                        };
                    }
                }
            }
        }
        let packet = hex(frame["ordinary"].as_str().unwrap());
        let boxes = cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes);
        let mut out = qmf
            .decode(&mut BitReader::new(&packet), &input, &vec![None; boxes])
            .unwrap();
        if n(&frame["temporal_config"]) == 1 {
            if stp.is_none() {
                stp = Some(Decoder::new(tree as u8, B).unwrap());
            }
            let envelope = targets(frame);
            stp.as_mut()
                .unwrap()
                .apply(&mut out.dry, &mut out.wet, envelope.as_deref())
                .unwrap();
        } else {
            stp = None;
        }
        let mut planar = Vec::new();
        for (ch, bank) in synthesis.iter_mut().enumerate() {
            let values: Vec<[Complex; B]> = (0..slots)
                .map(|t| {
                    std::array::from_fn(|b| Complex {
                        re: out.dry[t][ch][b].re + out.wet[t][ch][b].re,
                        im: out.dry[t][ch][b].im + out.wet[t][ch][b].im,
                    })
                })
                .collect();
            planar.push(bank.process(&values).unwrap());
        }
        let order = native_order(tree as u8).unwrap();
        for t in 0..slots * B {
            for wave in 0..outputs {
                let ch = if matches!(tree, 4 | 6) {
                    [0, 1, 2, 3, 6, 7, 4, 5][wave]
                } else {
                    wave
                };
                result.push(planar[order[ch]][t]);
            }
        }
    }
    result
}
#[test]
fn whole_native_stp_pcm_matches_separate_qualified_chain_and_config_transitions() {
    let golden = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let mut maximum = 0f64;
    let mut count = 0;
    for c in manifest("manifest.json")["cases"].as_array().unwrap() {
        let expected = if n(&c["bands"]) == 32 {
            reference::<32>(c, &golden)
        } else {
            reference::<64>(c, &golden)
        };
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap(),
            &mut pcm,
        )
        .unwrap();
        assert_eq!(pcm.len(), expected.len() * 4);
        for (b, want) in pcm.chunks_exact(4).zip(expected) {
            let got = f64::from(f32::from_le_bytes(b.try_into().unwrap()));
            maximum = maximum.max((got - want).abs());
            count += 1;
            assert!((got - want).abs() < 2e-7, "{}: {got} != {want}", c["name"]);
        }
    }
    eprintln!("STP whole-chain samples {count}, maximum native/separate difference {maximum}");
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}
#[test]
fn stp_movies_play_rewind_seek_and_repeat_ranges_with_the_same_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    for c in manifest("manifest.json")["cases"].as_array().unwrap() {
        let bytes =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut full).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let mut live = reader.make_decoder().unwrap();
        let width = usize::from(reader.channels()) * 4;
        assert_eq!(play(&mut reader, live.as_mut()), full);
        reader.rewind();
        live.reset();
        for _ in 0..4 {
            let p = reader.next_packet().unwrap().unwrap();
            live.decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap();
        }
        reader.rewind();
        live.reset();
        assert_eq!(
            play(&mut reader, live.as_mut()),
            full,
            "{} reset",
            c["name"]
        );
        let frame = n(&c["slots"]) * n(&c["bands"]);
        for target in [13, (4 * frame + 7) as i64, 7, (full.len() / width) as i64] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            live.reset();
            assert_eq!(
                play(&mut reader, live.as_mut()),
                full[first..],
                "{} seek {target}",
                c["name"]
            );
        }
        let rate = u64::from(reader.sample_rate());
        let ms = frame as u64 * 1000 / rate;
        for (from, to) in [(1, 7), (3 * ms - 2, 6 * ms + 3), (1, 7)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &bytes,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = ((from * rate).div_ceil(1000) as usize * width).min(full.len());
            let last = ((to * rate).div_ceil(1000) as usize * width).min(full.len());
            assert_eq!(range, full[first..last], "{} ranges", c["name"]);
        }
    }
}
#[test]
fn late_trailing_bytes_preserve_stp_core_and_source_histories() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest("manifest.json");
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for bad in m["errors"].as_array().unwrap() {
        let name = bad["name"].as_str().unwrap().split("-bad-").next().unwrap();
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let bytes =
            std::fs::read(root().join(format!("{}.mp4", bad["name"].as_str().unwrap()))).unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{}: {error}",
            bad["name"]
        );
        let asc = hex(good["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut reference = NativeAacDecoder::new(&asc).unwrap();
        let frame = n(&good["slots"]) * n(&good["bands"]);
        for i in 0..12 {
            let row = &good["frames"][i];
            let at = n(&row["offset"]);
            let packet = &blob[at..at + n(&row["bytes"])];
            let expected = reference
                .decode_timed(packet, (i * frame) as i64, frame as u64)
                .unwrap();
            if i == 3 {
                let saved = decoder.checkpoint();
                let before = decoder.retained_payload_bytes().unwrap();
                let row = &bad["frames"][i];
                let at = n(&row["offset"]);
                let error = decoder
                    .decode_timed(
                        &blob[at..at + n(&row["bytes"])],
                        (i * frame) as i64,
                        frame as u64,
                    )
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                assert_eq!(decoder.retained_payload_bytes().unwrap(), before);
                assert_eq!(
                    decoder
                        .decode_timed(packet, (i * frame) as i64, frame as u64)
                        .unwrap(),
                    expected
                );
                decoder.restore(&saved).unwrap();
            }
            assert_eq!(
                decoder
                    .decode_timed(packet, (i * frame) as i64, frame as u64)
                    .unwrap(),
                expected
            );
        }
        assert_eq!(decoder.finish().unwrap(), reference.finish().unwrap());
        assert!(decoder.finish().unwrap().is_none());
    }
}

#[test]
fn independent_adapted_iso_stp_component_matches_all_trees_and_preserves_state() {
    let m = manifest("component.json");
    let inputs = std::fs::read(root().join("component-input.f32le")).unwrap();
    let expected = std::fs::read(root().join("component-expected.f32le")).unwrap();
    let mut count = 0;
    let mut maximum = 0f64;
    let mut changed = 0;
    let signal = |at: &mut usize, slots: usize, channels: usize, bands: usize| {
        (0..slots)
            .map(|_| {
                (0..channels)
                    .map(|_| {
                        (0..bands)
                            .map(|_| {
                                let re = f64::from(f32::from_le_bytes(
                                    inputs[*at..*at + 4].try_into().unwrap(),
                                ));
                                let im = f64::from(f32::from_le_bytes(
                                    inputs[*at + 4..*at + 8].try_into().unwrap(),
                                ));
                                *at += 8;
                                Complex { re, im }
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    for c in m["cases"].as_array().unwrap() {
        let tree = n(&c["tree"]) as u8;
        let bands = n(&c["bands"]);
        let channels = n(&c["outputs"]);
        let mut decoder = Decoder::new(tree, bands).unwrap();
        let fresh = decoder.clone();
        for block in c["blocks"].as_array().unwrap() {
            let slots = n(&block["slots"]);
            let mut at = n(&block["offset"]);
            let mut dry = signal(&mut at, slots, channels, bands);
            let mut wet = signal(&mut at, slots, channels, bands);
            let original_dry = dry.clone();
            let original_wet = wet.clone();
            let flags = block["flags"]
                .as_array()
                .map(|r| r.iter().map(|b| b.as_bool().unwrap()).collect::<Vec<_>>());
            let before = decoder.clone();
            let mut chunked = decoder.clone();
            let mut chunk_dry = Vec::new();
            let mut chunk_wet = Vec::new();
            for t in 0..slots {
                let mut d = vec![dry[t].clone()];
                let mut w = vec![wet[t].clone()];
                chunked.apply(&mut d, &mut w, flags.as_deref()).unwrap();
                chunk_dry.extend(d);
                chunk_wet.extend(w);
            }
            decoder.apply(&mut dry, &mut wet, flags.as_deref()).unwrap();
            assert_eq!(decoder, chunked);
            assert_eq!(dry, chunk_dry);
            assert_eq!(wet, chunk_wet);
            let mut at = n(&block["expected_offset"]);
            for t in 0..slots {
                for ch in 0..channels {
                    for b in 0..bands {
                        let got = Complex {
                            re: dry[t][ch][b].re + wet[t][ch][b].re,
                            im: dry[t][ch][b].im + wet[t][ch][b].im,
                        };
                        for actual in [got.re, got.im] {
                            let want = f64::from(f32::from_le_bytes(
                                expected[at..at + 4].try_into().unwrap(),
                            ));
                            at += 4;
                            maximum = maximum.max((actual - want).abs());
                            count += 1;
                            assert!(
                                (actual - want).abs() < 2e-7,
                                "tree{tree} B{bands} t{t} ch{ch} b{b}: {actual} vs {want}"
                            );
                        }
                        let plain = Complex {
                            re: original_dry[t][ch][b].re + original_wet[t][ch][b].re,
                            im: original_dry[t][ch][b].im + original_wet[t][ch][b].im,
                        };
                        if flags.is_none() || b < 12 {
                            assert_eq!(got, plain);
                        }
                        if b < 12 {
                            assert_eq!(wet[t][ch][b], Complex::default());
                        }
                        if (got.re - plain.re).abs() > 1e-8 {
                            changed += 1;
                        }
                    }
                }
            }
            let mut retry = before.clone();
            let mut d = original_dry.clone();
            let mut bad = original_wet.clone();
            bad[slots - 1][channels - 1][bands - 1].re = f64::NAN;
            let retained = d.clone();
            assert!(retry.apply(&mut d, &mut bad, flags.as_deref()).is_err());
            assert_eq!(retry, before);
            assert_eq!(d, retained);
            // Force a late overflow in bin0, outside the energy summation.
            // All slots have advanced history by the final finite-output check.
            let mut overflow_dry = original_dry.clone();
            let mut overflow_wet = original_wet.clone();
            overflow_dry[slots - 1][0][0].re = f64::MAX;
            overflow_wet[slots - 1][0][0].re = f64::MAX;
            let retained_dry = overflow_dry.clone();
            let retained_wet = overflow_wet.clone();
            assert!(
                retry
                    .apply(&mut overflow_dry, &mut overflow_wet, flags.as_deref())
                    .is_err()
            );
            assert_eq!(retry, before);
            assert_eq!(overflow_dry, retained_dry);
            assert_eq!(overflow_wet, retained_wet);
            let mut d = original_dry;
            let mut w = original_wet;
            retry.apply(&mut d, &mut w, flags.as_deref()).unwrap();
            assert_eq!(retry, decoder);
            assert_eq!(d, dry);
            assert_eq!(w, wet);
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        assert!(decoder.retained_payload_bytes().unwrap() < 1024);
    }
    assert_eq!(count, 1_038_336);
    assert!(changed > 10_000);
    eprintln!(
        "STP independent component scalars {count}, maximum difference {maximum}, changed {changed}"
    );
}
