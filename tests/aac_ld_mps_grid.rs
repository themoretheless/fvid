use fvid_media::owned_aac::{
    NativeAacDecoder,
    aac_ld_mps_config::Config,
    aac_ld_mps_matrix_time::{Layout, MatrixFrame, MatrixTime},
    aac_ld_mps_prepare::SmoothControl,
    bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-grid")
}
fn json(name: &str) -> Value {
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
fn movie(c: &Value) -> Vec<u8> {
    std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap()
}
#[test]
fn native_resolution_change_movies_accept_correct_header_and_frame_pairs() {
    let m = json("manifest.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 560);
    for c in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let cfg = Config::parse(&hex(c["configs"][0].as_str().unwrap())).unwrap();
        assert_eq!(
            pcm.len(),
            n(&c["samples"]) * usize::from(cfg.tree.output_channels) * 4
        );
        assert!(pcm.iter().any(|&b| b != 0));
    }
}
#[test]
fn indexed_active_and_dormant_history_matches_exact_fraction_oracle() {
    let m = json("matrix-history.json");
    let mut checked = 0;
    for c in m["cases"].as_array().unwrap() {
        let width = n(&c["width"]);
        let layout = Layout {
            planes: 1,
            rows: if width == 12 { 2 } else { 1 },
            columns: if width == 12 { 6 } else { width as u8 },
        };
        let mut time = MatrixTime::new(n(&c["frames"][0]["bands"]) as u8, layout).unwrap();
        let fresh = time.clone();
        for f in c["frames"].as_array().unwrap() {
            time.reconfigure_bands(n(&f["bands"]) as u8).unwrap();
            let saved = time.clone();
            let mut invalid = time.clone();
            assert!(invalid.reconfigure_bands(0).is_err());
            assert_eq!(invalid, time);
            assert!(invalid.reconfigure_bands(24).is_err());
            assert_eq!(invalid, time);
            let endpoints = f["endpoints"].as_array().unwrap();
            let slots: Vec<_> = endpoints.iter().map(|p| n(&p["slot"]) as u8).collect();
            let controls: Vec<_> = endpoints
                .iter()
                .map(|p| SmoothControl {
                    time: n(&p["time"]) as u16,
                    mask: p["mask"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|b| b.as_bool().unwrap())
                        .collect(),
                })
                .collect();
            let raw: Vec<Vec<f64>> = endpoints
                .iter()
                .map(|p| {
                    p["raw"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_f64().unwrap())
                        .collect()
                })
                .collect();
            let output = time.process(8, &slots, &controls, &raw, 0).unwrap();
            for (actual, endpoint) in output.endpoints.iter().zip(endpoints) {
                let expected: Vec<_> = endpoint["expected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap())
                    .collect();
                assert_eq!(actual, &expected);
                checked += actual.len();
            }
            let first: Vec<_> = f["first_slot"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            assert_eq!(output.slots[0], first);
            let mut replay = saved.clone();
            assert_eq!(
                replay.process(8, &slots, &controls, &raw, 0).unwrap(),
                output
            );
            assert_eq!(replay, time);
            let mut failed = saved.clone();
            let mut bad = raw;
            bad[1][0] = f64::NAN;
            assert!(failed.process(8, &slots, &controls, &bad, 0).is_err());
            assert_eq!(failed, saved);
        }
        time.reset();
        time.reconfigure_bands(n(&c["frames"][0]["bands"]) as u8)
            .unwrap();
        assert_eq!(time, fresh);
    }
    eprintln!("exact indexed matrix-history scalar endpoints: {checked}");
}
fn reference<const B: usize>(c: &Value, raw_meta: &Value, raw: &[u8], golden: &[u8]) -> Vec<f64> {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::{Decorrelator, Mode},
        aac_ld_mps_matrix_apply::apply,
        aac_ld_mps_pcm::native_order,
        aac_ld_mps_qmf::seeds,
        aac_ld_mps_route::Router,
        aac_ld_qmf::{Analysis, Synthesis},
        aac_sbr_qmf::Complex,
    };
    let tree = n(&c["tree"]);
    let channels = n(&c["channels"]);
    let slots = n(&c["slots"]);
    let virtuals = if tree < 2 {
        5
    } else if tree == 2 {
        6
    } else if tree == 7 {
        2
    } else {
        8
    };
    let direct = if matches!(tree, 0 | 1 | 7) {
        1
    } else if matches!(tree, 2..=4) {
        3
    } else {
        6
    };
    let outputs = if tree == 7 {
        2
    } else if tree < 3 {
        6
    } else {
        8
    };
    let widths = [
        virtuals * direct * 2,
        outputs * virtuals,
        outputs * virtuals,
    ];
    let l1 = Layout {
        planes: 2,
        rows: virtuals as u8,
        columns: direct as u8,
    };
    let l2 = Layout {
        planes: 1,
        rows: outputs as u8,
        columns: virtuals as u8,
    };
    let mut history: Vec<Vec<f64>> = widths.iter().map(|w| vec![0.; 23 * w]).collect();
    let mut decor: Vec<_> = seeds(tree as u8)
        .unwrap()
        .iter()
        .map(|&seed| Decorrelator::new(B, seed, 0, Mode::Complex).unwrap())
        .collect();
    let mut analysis: Vec<_> = (0..channels)
        .map(|_| Analysis::<B>::new().unwrap())
        .collect();
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
    for frame in 0..3 {
        let cfg = Config::parse(&hex(c["configs"][frame].as_str().unwrap())).unwrap();
        let bands = usize::from(cfg.parameter_bands);
        assert_eq!(cfg.downmix_gain, 0);
        assert!(cfg.surround_gain.unwrap_or(0) == 0 && cfg.lfe_gain.unwrap_or(0) == 0);
        let entry = raw_meta["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["source"] == c["sources"][frame])
            .unwrap();
        assert_eq!(n(&entry["bands"]), bands);
        let mut matrices: Vec<_> = (0..3)
            .map(|_| MatrixFrame {
                endpoints: Vec::new(),
                slots: Vec::new(),
            })
            .collect();
        let mut last = -1i32;
        for (ps, slot) in [slots.div_ceil(2) - 1, slots - 1].into_iter().enumerate() {
            for plane in 0..3 {
                let width = widths[plane];
                let mut end = Vec::new();
                for band in 0..bands {
                    for j in 0..width {
                        let at = n(&entry["offset"])
                            + ((ps * bands + band) * widths.iter().sum::<usize>()
                                + widths[..plane].iter().sum::<usize>()
                                + j)
                                * 8;
                        let raw = f64::from_le_bytes(raw[at..at + 8].try_into().unwrap());
                        let prev = history[plane][band * width + j];
                        let delta = (slot as i32 - last) as f64 / 128.;
                        end.push(if ps == 1 {
                            delta * raw + (1. - delta) * prev
                        } else {
                            raw
                        });
                    }
                }
                for t in last + 1..=slot as i32 {
                    let alpha = (t - last) as f64 / (slot as i32 - last) as f64;
                    matrices[plane].slots.push(
                        end.iter()
                            .enumerate()
                            .map(|(i, &v)| (1. - alpha) * history[plane][i] + alpha * v)
                            .collect(),
                    );
                }
                history[plane][..bands * width].copy_from_slice(&end);
            }
            last = slot as i32;
        }
        let mut input = vec![vec![vec![Complex::default(); B]; channels]; slots];
        if c["qmf"].is_null() {
            let decoded = core
                .decode(
                    &std::fs::read(parent.join(format!("aac-eld-mps-core/tree{tree}-{frame}.au")))
                        .unwrap(),
                )
                .unwrap();
            for ch in 0..channels {
                let pcm: Vec<_> = decoded
                    .chunks_exact(channels)
                    .map(|r| f64::from(r[ch]))
                    .collect();
                for (t, row) in analysis[ch].process(&pcm).unwrap().into_iter().enumerate() {
                    input[t][ch] = row.to_vec();
                }
            }
        } else {
            let offset = n(&c["qmf"]["expected_offset"]);
            let stored = n(&c["qmf"]["expected_bands"]);
            for t in 0..slots {
                for ch in 0..channels {
                    for b in 0..B.min(stored) {
                        let at = offset + ((ch * 12 * slots + frame * slots + t) * stored + b) * 16;
                        input[t][ch][b] = Complex {
                            re: f64::from_le_bytes(golden[at..at + 8].try_into().unwrap()),
                            im: f64::from_le_bytes(golden[at + 8..at + 16].try_into().unwrap()),
                        };
                    }
                }
            }
        }
        if cfg.tree.ttt_boxes != 0 {
            for slot in &mut input {
                slot.push(vec![Complex::default(); B]);
            }
        }
        let mixed = apply(&matrices[0], l1, cfg.parameter_bands, B, 0, Some(1), &input).unwrap();
        let mut wet = vec![vec![vec![Complex::default(); B]; virtuals]; slots];
        for (j, bank) in decor.iter_mut().enumerate() {
            let source: Vec<_> = mixed.iter().map(|s| s[direct + j].clone()).collect();
            for (t, row) in bank.process(&source).unwrap().into_iter().enumerate() {
                wet[t][direct + j] = row;
            }
        }
        let modes = if let Some(ttt) = cfg.ttt.first() {
            (0..cfg.parameter_bands)
                .map(|b| {
                    ttt.high
                        .filter(|&(_, split)| b >= split)
                        .map_or(ttt.low_mode, |(m, _)| m)
                })
                .collect()
        } else {
            Vec::new()
        };
        let boxes = cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes);
        let router =
            Router::new(tree as u8, cfg.parameter_bands, B, vec![0; boxes], modes).unwrap();
        let routed = router.process(&mixed, &wet, &vec![None; boxes]).unwrap();
        let dry = apply(
            &matrices[1],
            l2,
            cfg.parameter_bands,
            B,
            0,
            None,
            &routed.dry,
        )
        .unwrap();
        let wet = apply(
            &matrices[2],
            l2,
            cfg.parameter_bands,
            B,
            0,
            None,
            &routed.wet,
        )
        .unwrap();
        let mut planar = Vec::new();
        for (ch, bank) in synthesis.iter_mut().enumerate() {
            let combined: Vec<[Complex; B]> = (0..slots)
                .map(|t| {
                    std::array::from_fn(|b| Complex {
                        re: dry[t][ch][b].re + wet[t][ch][b].re,
                        im: dry[t][ch][b].im + wet[t][ch][b].im,
                    })
                })
                .collect();
            planar.push(bank.process(&combined).unwrap());
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
fn native_pcm_matches_independent_iso_matrices_and_indexed_temporal_reference() {
    let meta = json("raw-matrices.json");
    let raw = std::fs::read(root().join("raw-matrices.f64le")).unwrap();
    let golden = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let mut maximum = 0f64;
    let mut samples = 0;
    for c in json("manifest.json")["cases"].as_array().unwrap() {
        let expected = if n(&c["bands"]) == 32 {
            reference::<32>(c, &meta, &raw, &golden)
        } else {
            reference::<64>(c, &meta, &raw, &golden)
        };
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut actual).unwrap();
        assert_eq!(actual.len(), expected.len() * 4);
        for (bytes, want) in actual.chunks_exact(4).zip(expected) {
            let got = f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
            let delta = (got - want).abs();
            maximum = maximum.max(delta);
            samples += 1;
            assert!(delta < 2e-7, "{}: {got} != {want}; {delta}", c["name"]);
        }
    }
    eprintln!("grid native/reference samples {samples}, maximum absolute difference {maximum}");
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut result = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                result.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            result.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    result
}
#[test]
fn every_grid_movie_plays_rewinds_seeks_and_matches_owned_pcm_and_ranges() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    for c in json("manifest.json")["cases"].as_array().unwrap() {
        let bytes = movie(c);
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
        for _ in 0..2 {
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
        for target in [13, (frame + 7) as i64, 7, (full.len() / width) as i64] {
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
        for (from, to) in [(1, 7), (ms - 2, ms * 2 + 3), (1, 7)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &bytes,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = ((from * rate).div_ceil(1000) as usize * width).min(full.len());
            let last = ((to * rate).div_ceil(1000) as usize * width).min(full.len());
            assert_eq!(range, full[first..last], "{} range", c["name"]);
        }
    }
}
