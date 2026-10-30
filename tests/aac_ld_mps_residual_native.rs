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
#[test]
fn native_residual_movies_accept_active_shaping_instead_of_refusing_pcm() {
    let m = manifest("manifest.json");
    assert_eq!(m["cases"].as_array().unwrap().len(), 397);
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
#[test]
fn whole_native_residual_pcm_matches_separate_qualified_chain_and_config_transitions() {
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
        let expected = reference(c, &golden);
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
    eprintln!("Residual whole-chain samples {count}, maximum native/separate difference {maximum}");
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
fn residual_movies_play_rewind_seek_and_repeat_ranges_with_the_same_pcm() {
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
fn malformed_runs_and_late_trailing_bytes_preserve_residual_core_and_source_histories() {
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
        loop {
            let actual = decoder.finish().unwrap();
            let expected = reference.finish().unwrap();
            assert_eq!(actual, expected);
            if actual.is_none() {
                break;
            }
        }
        assert!(decoder.finish().unwrap().is_none());
    }
}

#[test]
fn unqualified_residual_banks_and_update8_are_explicit_refusals() {
    for c in manifest("manifest.json")["refusals"].as_array().unwrap() {
        let movie =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
    }
}

#[test]
fn source_window_session_matches_the_separate_residual_controls() {
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
    // The previous native check failed on this exact valid own payload.
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
    let mut native = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap(),
        &mut native,
    )
    .unwrap();
    for frame in 0..12 {
        let first = frame * slots * 64 * 2;
        let end = first + slots * 64 * 2;
        let mut gap = 0f64;
        for (actual, want) in native[first * 4..end * 4]
            .chunks_exact(4)
            .zip(&output[first..end])
        {
            gap = gap.max((f64::from(f32::from_le_bytes(actual.try_into().unwrap())) - want).abs());
        }
        assert!(gap < 2e-7, "front-end/session frame{frame}: {gap}");
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
fn residual_signal_and_icc_correction_both_change_the_pcm() {
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
