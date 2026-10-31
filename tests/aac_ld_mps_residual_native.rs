// Numerical residual compositions are not a permitted LD-MPS codec profile.
use fvid_media::owned_aac::aac_sbr_qmf::Complex;
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-residual-native")
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
fn reconfigured_reference(c: &Value, golden: &[u8]) -> Vec<f64> {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::{Config, ExtensionValue},
        aac_ld_mps_decor::Mode,
        aac_ld_mps_frame, aac_ld_mps_pcm, aac_ld_mps_residual, aac_mps_mdct_qmf,
        bits::BitReader,
    };
    let mut current = Config::parse(&hex(c["ssc"].as_str().unwrap())).unwrap();
    let initial_control = Config::parse(&hex(c["control_ssc"].as_str().unwrap())).unwrap();
    let mut pcm = aac_ld_mps_pcm::Decoder::new_qmf(&initial_control, Mode::Complex).unwrap();
    let mut syntax = aac_ld_mps_frame::Decoder::new(&current).unwrap();
    let mut banks = (0..c["cutoffs"].as_array().unwrap().len())
        .map(|_| None::<aac_mps_mdct_qmf::Decoder>)
        .collect::<Vec<_>>();
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    let outputs = usize::from(current.tree.output_channels);
    let mut result = Vec::new();
    for (i, frame) in c["reference"].as_array().unwrap().iter().enumerate() {
        let cfg = Config::parse(&hex(frame["ssc"].as_str().unwrap())).unwrap();
        if cfg != current {
            syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
            current = cfg.clone();
        }
        let control = Config::parse(&hex(frame["control_ssc"].as_str().unwrap())).unwrap();
        pcm.reconfigure(&control).unwrap();
        let input_offset = n(&c["qmf"]["expected_offset"]);
        let stored = n(&c["qmf"]["expected_bands"]);
        let mut input = vec![vec![vec![Complex::default(); 64]; inputs]; slots];
        for t in 0..slots {
            for ch in 0..inputs {
                for b in 0..64.min(stored) {
                    let at = input_offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                    input[t][ch][b] = Complex {
                        re: f64::from_le_bytes(golden[at..at + 8].try_into().unwrap()),
                        im: f64::from_le_bytes(golden[at + 8..at + 16].try_into().unwrap()),
                    };
                }
            }
        }
        for (bank, cutoff) in banks.iter_mut().zip(frame["cutoffs"].as_array().unwrap()) {
            if n(cutoff) == 0 {
                *bank = None;
            } else if bank.is_none() {
                *bank = Some(aac_mps_mdct_qmf::Decoder::new(slots, 1).unwrap());
            }
        }
        let parsed = syntax
            .decode(&mut BitReader::new(&hex(frame["spatial"]
                .as_str()
                .unwrap())))
            .unwrap();
        let mut residuals = vec![None; banks.len()];
        if let Some(extension) = cfg
            .extensions
            .iter()
            .find(|e| matches!(&e.value, ExtensionValue::Residual { .. }))
        {
            let decoded = aac_ld_mps_residual::read(
                &mut BitReader::new(&parsed.extensions[0].payload),
                &cfg,
                &extension.value,
                parsed.slots.len(),
            )
            .unwrap();
            for (row, (bank, data)) in banks.iter_mut().zip(&decoded.boxes).enumerate() {
                if let (Some(bank), Some(data)) = (bank, data) {
                    residuals[row] = Some(bank.process(&data.blocks).unwrap());
                }
            }
        }
        let frame = pcm
            .decode_qmf(
                &mut BitReader::new(&hex(frame["control"].as_str().unwrap())),
                &input,
                &residuals,
            )
            .unwrap();
        for row in frame.samples.chunks_exact(outputs) {
            for wave in 0..outputs {
                let ch = if matches!(cfg.tree.code, 4 | 6) {
                    [0, 1, 2, 3, 6, 7, 4, 5][wave]
                } else {
                    wave
                };
                result.push(row[ch]);
            }
        }
    }
    result
}
fn reference(c: &Value, golden: &[u8]) -> Vec<f64> {
    reference_control(c, golden, true, "control")
}
fn reference_control(c: &Value, golden: &[u8], signal: bool, control_key: &str) -> Vec<f64> {
    if c["mode"] == "cutoff" || c["mode"] == "mask" {
        return reconfigured_reference(c, golden);
    }
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_frame, aac_ld_mps_ges,
        aac_ld_mps_pcm::native_order, aac_ld_mps_qmf, aac_ld_mps_residual, aac_ld_mps_stp,
        aac_ld_qmf::Synthesis, aac_mps_mdct_qmf, bits::BitReader,
    };
    let cfg = Config::parse(&hex(c["ssc"].as_str().unwrap())).unwrap();
    let control = Config::parse(&hex(c["control_ssc"].as_str().unwrap())).unwrap();
    let extension = &cfg.extensions[0].value;
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    let outputs = usize::from(cfg.tree.output_channels);
    let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
    let mut qmf = aac_ld_mps_qmf::Decoder::new(&control, Mode::Complex).unwrap();
    let mut residuals = c["cutoffs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            if n(v) > 0 {
                Some(aac_mps_mdct_qmf::Decoder::new(slots, 1).unwrap())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut synthesis = (0..outputs)
        .map(|_| Synthesis::<64>::new().unwrap())
        .collect::<Vec<_>>();
    let mut stp = if c["mode"] == "stp" {
        Some(aac_ld_mps_stp::Decoder::new(cfg.tree.code, 64).unwrap())
    } else {
        None
    };
    let mut residual = if c["mode"] == "ges" {
        Some(aac_ld_mps_ges::Decoder::new(cfg.tree.code, 64).unwrap())
    } else {
        None
    };
    let mut result = Vec::new();
    for (i, frame) in c["reference"].as_array().unwrap().iter().enumerate() {
        let offset = n(&c["qmf"]["expected_offset"]);
        let stored = n(&c["qmf"]["expected_bands"]);
        let mut input = vec![vec![vec![Complex::default(); 64]; inputs]; slots];
        for t in 0..slots {
            for ch in 0..inputs {
                for b in 0..64.min(stored) {
                    let at = offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                    input[t][ch][b] = Complex {
                        re: f64::from_le_bytes(golden[at..at + 8].try_into().unwrap()),
                        im: f64::from_le_bytes(golden[at + 8..at + 16].try_into().unwrap()),
                    };
                }
            }
        }
        let spatial = hex(frame["spatial"].as_str().unwrap());
        let parsed = syntax.decode(&mut BitReader::new(&spatial)).unwrap();
        let decoded = aac_ld_mps_residual::read(
            &mut BitReader::new(&parsed.extensions[0].payload),
            &cfg,
            extension,
            parsed.slots.len(),
        )
        .unwrap();
        let mut rows = Vec::new();
        for (bank, data) in residuals.iter_mut().zip(&decoded.boxes) {
            rows.push(match (bank, data) {
                (Some(b), Some(d)) => Some(b.process(&d.blocks).unwrap()),
                (None, None) => None,
                _ => panic!("residual mask mismatch"),
            });
        }
        if !signal {
            for row in rows.iter_mut().flatten() {
                for slot in row {
                    slot.fill(Complex::default());
                }
            }
        }
        let encoded = hex(frame[control_key].as_str().unwrap());
        let mut out = qmf
            .decode(&mut BitReader::new(&encoded), &input, &rows)
            .unwrap();
        if let Some(stp) = &mut stp {
            let flags = frame["targets"]
                .as_array()
                .map(|r| r.iter().map(|v| v.as_bool().unwrap()).collect::<Vec<_>>());
            stp.apply(&mut out.dry, &mut out.wet, flags.as_deref())
                .unwrap();
        }
        if let Some(residual) = &mut residual {
            let env = frame["targets"].as_array().map(|r| {
                r.iter()
                    .map(|v| {
                        v.as_array()
                            .map(|a| a.iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>())
                    })
                    .collect::<Vec<_>>()
            });
            residual
                .apply(&input, &mut out.dry, &out.wet, env.as_deref())
                .unwrap();
        }
        let mut planar = Vec::new();
        for (ch, bank) in synthesis.iter_mut().enumerate() {
            let combined = (0..slots)
                .map(|t| {
                    std::array::from_fn(|b| Complex {
                        re: out.dry[t][ch][b].re + out.wet[t][ch][b].re,
                        im: out.dry[t][ch][b].im + out.wet[t][ch][b].im,
                    })
                })
                .collect::<Vec<[Complex; 64]>>();
            planar.push(bank.process(&combined).unwrap());
        }
        let order = native_order(cfg.tree.code).unwrap();
        for t in 0..slots * 64 {
            for wave in 0..outputs {
                let ch = if matches!(cfg.tree.code, 4 | 6) {
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

const PROFILE_ERROR: &str = "residual coding is not permitted in LD-MPS";
fn input(c: &Value, i: usize, gold: &[u8]) -> Vec<Vec<Vec<Complex>>> {
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    let stored = n(&c["qmf"]["expected_bands"]);
    let offset = n(&c["qmf"]["expected_offset"]);
    (0..slots)
        .map(|t| {
            (0..inputs)
                .map(|ch| {
                    (0..64)
                        .map(|b| {
                            if b >= stored {
                                Complex::default()
                            } else {
                                let at =
                                    offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                                Complex {
                                    re: f64::from_le_bytes(gold[at..at + 8].try_into().unwrap()),
                                    im: f64::from_le_bytes(
                                        gold[at + 8..at + 16].try_into().unwrap(),
                                    ),
                                }
                            }
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}
fn message(row: &Value) -> fvid_media::owned_aac::aac_ld_mps_transport::Message {
    use fvid_media::owned_aac::aac_ld_mps_transport::{Extension, Kind, Message};
    let header = row["header"].as_bool().unwrap();
    let mut payload = hex(row["spatial"].as_str().unwrap());
    if header {
        let cfg = hex(row["ssc"].as_str().unwrap());
        assert!(cfg.len() < 127);
        let mut prefix = vec![cfg.len() as u8];
        prefix.extend(cfg);
        prefix.extend(payload);
        payload = prefix;
    }
    Message {
        extension: Extension::LowDelay,
        kind: if header {
            Kind::HeaderAndFrame
        } else {
            Kind::Frame
        },
        payload,
    }
}
#[test]
fn native_codec_rejects_all_nonconforming_ld_residual_movies_before_pcm() {
    let m = manifest("manifest.json");
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["errors"].as_array().unwrap())
        .chain(m["refusals"].as_array().unwrap())
    {
        let data =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let mut p = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(&data, &mut p).unwrap_err();
        assert!(e.to_string().contains(PROFILE_ERROR), "{} {e}", c["name"]);
        assert!(p.is_empty());
    }
}
#[test]
fn controlled_session_composition_matches_separate_stages_without_codec_profile_claim() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config,
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
    };
    let gold = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    for c in manifest("manifest.json")["cases"].as_array().unwrap() {
        let cfg = Config::parse(&hex(c["ssc"].as_str().unwrap())).unwrap();
        assert!(cfg.validate_low_delay_profile().is_err());
        let mut session = Session::new_qmf(&cfg, Mode::Complex).unwrap();
        let mut actual = Vec::new();
        let samples = n(&c["slots"]) * 64;
        let width = usize::from(cfg.tree.output_channels);
        for (i, row) in c["reference"].as_array().unwrap().iter().enumerate() {
            let qmf = input(c, i, &gold);
            let frames = session
                .submit_qmf(
                    Some((
                        &qmf,
                        Stamp {
                            pts: (i * samples) as i64,
                            duration: samples as u64,
                            timescale: 48000,
                        },
                    )),
                    &[message(row)],
                )
                .unwrap();
            for frame in frames {
                for row in frame.pcm.samples.chunks_exact(width) {
                    for wave in 0..width {
                        let ch = if matches!(cfg.tree.code, 4 | 6) {
                            [0, 1, 2, 3, 6, 7, 4, 5][wave]
                        } else {
                            wave
                        };
                        actual.push(row[ch]);
                    }
                }
            }
        }
        session.finish().unwrap();
        let want = reference(c, &gold);
        assert_eq!(actual.len(), want.len());
        for (a, b) in actual.iter().zip(want) {
            let gap = (a - b).abs();
            maximum = maximum.max(gap);
            assert!(gap < 1e-9, "{} math {gap}", c["name"]);
            count += 1;
        }
    }
    eprintln!(
        "NONCONFORMING numerical session/control values {count};max {maximum};not codec acceptance"
    );
}
#[test]
fn malformed_residual_math_remains_transactional_without_a_native_codec_profile() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config,
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
    };
    let m = manifest("manifest.json");
    let gold = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    for c in m["errors"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| {
                c["name"]
                    .as_str()
                    .unwrap()
                    .starts_with(g["name"].as_str().unwrap())
                    && g["mode"] == "plain"
            })
            .unwrap_or_else(|| {
                m["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|g| {
                        g["name"]
                            .as_str()
                            .unwrap()
                            .starts_with("n512-c1-base-x2-crc1-v1-bands23-plain")
                    })
                    .unwrap()
            });
        let cfg = Config::parse(&hex(good["ssc"].as_str().unwrap())).unwrap();
        let mut d = Session::new_qmf(&cfg, Mode::Complex).unwrap();
        let mut control = d.clone();
        let samples = n(&good["slots"]) * 64;
        for (i, row) in good["reference"].as_array().unwrap().iter().enumerate() {
            let qmf = input(good, i, &gold);
            let stamp = Stamp {
                pts: (i * samples) as i64,
                duration: samples as u64,
                timescale: 48000,
            };
            let want = control
                .submit_qmf(Some((&qmf, stamp)), &[message(row)])
                .unwrap();
            if i == 3 {
                let before = d.clone();
                let e = d
                    .submit_qmf(Some((&qmf, stamp)), &[message(&c["reference"][i])])
                    .unwrap_err();
                assert!(
                    e.to_string().contains(c["error"].as_str().unwrap()),
                    "{} {e}",
                    c["name"]
                );
                assert_eq!(d, before);
            }
            assert_eq!(
                d.submit_qmf(Some((&qmf, stamp)), &[message(row)]).unwrap(),
                want
            );
        }
        d.finish().unwrap();
    }
}
#[test]
fn controlled_source_window_math_matches_separate_residual_stages() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config,
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
        aac_ld_mps_transport::{Kind, Message},
    };
    let manifest = manifest("manifest.json");
    let c = &manifest["cases"][0];
    let golden = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let cfg = Config::parse(&hex(c["ssc"].as_str().unwrap())).unwrap();
    let asc =
        fvid_media::owned_aac::config::AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
            .unwrap();
    assert_eq!(asc.ld_mps.as_ref(), Some(&cfg));
    assert_eq!(c["mode"], "fill");
    // The bounded extension permits nonzero fill bits and complete fill bytes.
    // This checks bounded syntax only;the LD codec profile forbids residual coding.
    let first = hex(c["reference"][1]["spatial"].as_str().unwrap());
    let raw = fvid_media::owned_aac::aac_ld_mps_frame::Decoder::new(&cfg)
        .unwrap()
        .decode(&mut fvid_media::owned_aac::bits::BitReader::new(&first))
        .unwrap();
    let mut fill = fvid_media::owned_aac::bits::BitReader::new(&raw.extensions[0].payload);
    fvid_media::owned_aac::aac_ld_mps_residual::read(
        &mut fill,
        &cfg,
        &cfg.extensions[0].value,
        raw.slots.len(),
    )
    .unwrap();
    assert!(fill.remaining() > 16);
    let mut nonzero = false;
    while fill.remaining() != 0 {
        nonzero |= fill.bit().unwrap();
    }
    assert!(nonzero);

    let mut session = Session::new_qmf(&cfg, Mode::Complex).unwrap();
    let mut output = Vec::new();
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    for (i, frame) in c["reference"].as_array().unwrap().iter().enumerate() {
        let offset = n(&c["qmf"]["expected_offset"]);
        let stored = n(&c["qmf"]["expected_bands"]);
        let mut input = vec![vec![vec![Complex::default(); 64]; inputs]; slots];
        for t in 0..slots {
            for ch in 0..inputs {
                for b in 0..64.min(stored) {
                    let at = offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                    input[t][ch][b] = Complex {
                        re: f64::from_le_bytes(golden[at..at + 8].try_into().unwrap()),
                        im: f64::from_le_bytes(golden[at + 8..at + 16].try_into().unwrap()),
                    };
                }
            }
        }
        let frames = session
            .submit_qmf(
                Some((
                    &input,
                    Stamp {
                        pts: (i * slots * 64) as i64,
                        duration: (slots * 64) as u64,
                        timescale: 48000,
                    },
                )),
                &[Message {
                    extension: fvid_media::owned_aac::aac_ld_mps_transport::Extension::LowDelay,
                    kind: Kind::Frame,
                    payload: hex(frame["spatial"].as_str().unwrap()),
                }],
            )
            .unwrap();
        output.extend(frames.into_iter().flat_map(|f| f.pcm.samples));
    }
    let expected = reference(c, &golden);
    let mut maximum = 0f64;
    let mut where_max = 0;
    for (i, (a, b)) in output.iter().zip(&expected).enumerate() {
        if (a - b).abs() > maximum {
            maximum = (a - b).abs();
            where_max = i;
        }
    }
    assert_eq!(output.len(), expected.len());
    assert!(
        maximum < 1e-12,
        "session/control max {maximum} at {where_max}: {} vs {}",
        output[where_max],
        expected[where_max]
    );
}

#[test]
fn controlled_residual_signal_and_icc_correction_both_change_the_pcm() {
    let golden = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let m = manifest("manifest.json");
    let mut least_signal = f64::INFINITY;
    let mut least_icc = f64::INFINITY;
    let mut cases = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "plain" && c["parameter_bands"] == 23)
    {
        cases += 1;
        let full = reference(c, &golden);
        let silent = reference_control(c, &golden, false, "control");
        let original = reference_control(c, &golden, true, "uncorrected_control");
        let distance = |v: &[f64]| {
            full.iter()
                .zip(v)
                .fold(0f64, |m, (a, b)| m.max((a - b).abs()))
        };
        let signal = distance(&silent);
        let icc = distance(&original);
        assert!(
            signal > 1e-7,
            "{} residual is inaudible to the comparison: {signal}",
            c["name"]
        );
        assert!(
            icc > 1e-7,
            "{} ICC correction is inactive: {icc}",
            c["name"]
        );
        least_signal = least_signal.min(signal);
        least_icc = least_icc.min(icc);
    }
    assert_eq!(cases, 22);
    eprintln!("Residual activity controls minimum signal {least_signal}, ICC {least_icc}");
}
