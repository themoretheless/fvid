//! Decorrelation/QMF stereo component acceptance, not full native PCM playback.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_decorrelation::{self as decor, FrameControls},
    aac_ps_history, aac_ps_hybrid,
    aac_ps_mapping::Bands,
    aac_ps_matrix_controller, aac_sbr_history,
    aac_sbr_qmf::Complex,
    bits::BitReader,
};
use serde_json::Value;
const DATA: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-decorrelation-reference.bin");
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-decorrelation-oracles.json"
    ))
    .unwrap()
}
fn bands(v: &Value) -> Bands {
    match v.as_u64().unwrap() {
        20 => Bands::Twenty,
        34 => Bands::ThirtyFour,
        _ => panic!(),
    }
}
fn scalars_from(data: &[u8], v: &Value) -> Vec<f64> {
    let off = v[0].as_u64().unwrap() as usize;
    let count = v[1].as_u64().unwrap() as usize;
    data[off..off + count * 8]
        .chunks_exact(8)
        .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
fn scalars(v: &Value) -> Vec<f64> {
    scalars_from(DATA, v)
}
fn input(row: &Value) -> Vec<Vec<Complex>> {
    let width = bands(&row["bands"]).bindings().len();
    scalars(&row["input"])
        .chunks_exact(width * 2)
        .map(|r| {
            r.chunks_exact(2)
                .map(|p| Complex { re: p[0], im: p[1] })
                .collect()
        })
        .collect()
}
fn controls(row: &Value) -> FrameControls {
    FrameControls {
        previous_ps_present: row["previous_ps_present"].as_bool().unwrap(),
        qmf_limit: row["qmf_limit"].as_u64().unwrap() as usize,
    }
}
fn close(a: f64, e: f64) {
    assert!(a.is_finite() && e.is_finite());
    assert!((a - e).abs() < 3e-11 * (1. + e.abs()), "{a} != {e}");
}
fn compare(actual: impl IntoIterator<Item = f64>, descriptor: &Value) {
    let a: Vec<_> = actual.into_iter().collect();
    let e = scalars(descriptor);
    assert_eq!(a.len(), e.len());
    for (a, e) in a.into_iter().zip(e) {
        close(a, e);
    }
}
fn check(frame: &decor::Frame, state: &decor::State, row: &Value) {
    assert_eq!(frame.bands, bands(&row["bands"]));
    assert_eq!(frame.output.len(), row["slots"].as_u64().unwrap() as usize);
    compare(
        frame.output.iter().flatten().flat_map(|c| [c.re, c.im]),
        &row["output"],
    );
    compare(
        frame
            .unattenuated
            .iter()
            .flatten()
            .flat_map(|c| [c.re, c.im]),
        &row["raw"],
    );
    compare(frame.power.iter().flatten().copied(), &row["power"]);
    compare(frame.gains.iter().flatten().copied(), &row["gains"]);
    compare(state.peak().iter().copied(), &row["peak"]);
    compare(state.smoothed_power().iter().copied(), &row["smooth"]);
    compare(
        state.smoothed_difference().iter().copied(),
        &row["difference"],
    );
    assert!(
        frame
            .gains
            .iter()
            .flatten()
            .all(|&g| (0.0..=1.0).contains(&g))
    );
}
#[test]
fn every_phase_feedback_center_and_delay_matches_normative_decimal_values() {
    let refs = reference();
    let rows = refs["coefficients"].as_array().unwrap();
    assert_eq!(rows.len(), 71 + 91);
    for row in rows {
        let grid = bands(&row["bands"]);
        let k = row["k"].as_u64().unwrap() as usize;
        let actual = decor::coefficients(grid, k).unwrap();
        if row["delay"].is_number() {
            assert!(actual.is_none());
            continue;
        }
        let c = actual.unwrap();
        assert_eq!(c.delays, [3, 4, 5]);
        let a = [
            c.center,
            c.decay,
            c.initial_phase.re,
            c.initial_phase.im,
            c.link_phases[0].re,
            c.link_phases[0].im,
            c.link_phases[1].re,
            c.link_phases[1].im,
            c.link_phases[2].re,
            c.link_phases[2].im,
            c.feedback[0],
            c.feedback[1],
            c.feedback[2],
        ];
        for (a, e) in a.into_iter().zip(row["values"].as_array().unwrap()) {
            close(a, e.as_f64().unwrap());
        }
    }
    for grid in [Bands::Twenty, Bands::ThirtyFour] {
        assert!(decor::coefficients(grid, grid.bindings().len()).is_err());
    }
}
#[test]
fn all_filter_samples_gains_powers_and_retained_transients_match_independent_closed_forms() {
    let refs = reference();
    let cases = refs["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 11);
    let mut suppressed = false;
    for case in cases {
        let mut state = decor::State::default();
        for row in case["frames"].as_array().unwrap() {
            let before = state.bands();
            let frame = state
                .process_frame(bands(&row["bands"]), controls(row), &input(row))
                .unwrap();
            assert_eq!(frame.bands_changed, before != frame.bands);
            check(&frame, &state, row);
            suppressed |= frame.gains.iter().flatten().any(|&g| g < 0.5);
        }
    }
    assert!(suppressed, "fixtures must exercise the attenuation branch");
}
#[test]
fn every_unattenuated_allpass_and_delay_preserves_complex_impulse_energy() {
    for case in reference()["cases"].as_array().unwrap() {
        if !case["name"].as_str().unwrap().ends_with("_impulse") {
            continue;
        }
        let row = &case["frames"][0];
        let samples = input(row);
        let grid = bands(&row["bands"]);
        let frame = decor::State::new(grid).process(grid, &samples).unwrap();
        for k in 0..grid.bindings().len() {
            let source = samples
                .iter()
                .map(|r| r[k].re * r[k].re + r[k].im * r[k].im)
                .sum::<f64>();
            let result = frame
                .unattenuated
                .iter()
                .map(|r| r[k].re * r[k].re + r[k].im * r[k].im)
                .sum::<f64>();
            assert!(source > 0.);
            assert!(
                (result / source - 1.).abs() < 5e-8,
                "impulse energy at band {k}: {result}/{source}"
            );
        }
    }
}
#[test]
fn frame_resets_chunking_checkpoints_and_empty_calls_preserve_causal_state() {
    for case in reference()["cases"].as_array().unwrap() {
        let mut complete = decor::State::default();
        let rows = case["frames"].as_array().unwrap();
        let frames: Vec<_> = rows
            .iter()
            .map(|r| {
                complete
                    .process_frame(bands(&r["bands"]), controls(r), &input(r))
                    .unwrap()
            })
            .collect();
        for width in [1, 2, 3, 4, 5, 7, 14, 24, 30, 32] {
            let mut state = decor::State::default();
            for (row, expected) in rows.iter().zip(&frames) {
                let grid = bands(&row["bands"]);
                let samples = input(row);
                let mut output = vec![];
                let mut raw = vec![];
                let mut gains = vec![];
                let mut power = vec![];
                for (n, chunk) in samples.chunks(width).enumerate() {
                    let before = state.clone();
                    let empty = state.process_frame(grid, controls(row), &[]).unwrap();
                    assert_eq!(empty.bands, before.bands());
                    assert!(empty.output.is_empty());
                    assert!(!empty.bands_changed);
                    assert_eq!(state, before);
                    let mut replay = before;
                    let c = if n == 0 {
                        controls(row)
                    } else {
                        FrameControls::default()
                    };
                    let f = state.process_frame(grid, c, chunk).unwrap();
                    assert_eq!(f, replay.process_frame(grid, c, chunk).unwrap());
                    assert_eq!(state, replay);
                    output.extend(f.output);
                    raw.extend(f.unattenuated);
                    gains.extend(f.gains);
                    power.extend(f.power);
                }
                assert_eq!(output, expected.output);
                assert_eq!(raw, expected.unattenuated);
                assert_eq!(gains, expected.gains);
                assert_eq!(power, expected.power);
            }
            assert_eq!(state, complete);
            for grid in [Bands::Twenty, Bands::ThirtyFour] {
                state.reset(grid);
                assert_eq!(state, decor::State::new(grid));
            }
        }
    }
}
#[test]
fn malformed_late_energy_and_control_failures_do_not_commit_resets_or_filter_state() {
    let refs = reference();
    let row = &refs["cases"][3]["frames"][0];
    let mut state = decor::State::default();
    state.process(Bands::Twenty, &input(row)).unwrap();
    let before = state.clone();
    for grid in [Bands::Twenty, Bands::ThirtyFour] {
        let count = grid.bindings().len();
        let c = FrameControls {
            previous_ps_present: false,
            qmf_limit: 3,
        };
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            let mut samples = vec![vec![Complex::default(); count]; 32];
            samples[31][count - 1].re = bad;
            assert!(state.process_frame(grid, c, &samples).is_err());
            assert_eq!(state, before);
        }
        assert!(
            state
                .process(grid, &[vec![Complex::default(); count - 1]])
                .is_err()
        );
        assert_eq!(state, before);
        assert!(
            state
                .process_frame(
                    grid,
                    FrameControls {
                        previous_ps_present: false,
                        qmf_limit: 65
                    },
                    &[]
                )
                .is_err()
        );
        assert_eq!(state, before);
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn original_mp4_drives_hybrid_decorrelation_and_complete_stereo_qmf_mixing() {
    let refs = reference();
    let case = refs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "hybrid_video")
        .unwrap();
    let name = case["video"].as_str().unwrap();
    let matrix: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-matrix-controller-oracles.json"
    ))
    .unwrap();
    let v = matrix["videos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["video"]["file"] == name)
        .unwrap();
    let bank: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-hybrid-oracles.json"
    ))
    .unwrap();
    let bc = bank["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "grid_video")
        .unwrap();
    let flat = scalars_from(
        include_bytes!("fixtures/playback-errors/aac-ps-hybrid-reference.bin"),
        &bc["input"],
    );
    let qmf: Vec<[Complex; 64]> = flat
        .chunks_exact(128)
        .map(|r| {
            std::array::from_fn(|k| Complex {
                re: r[k * 2],
                im: r[k * 2 + 1],
            })
        })
        .collect();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name);
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let ai = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let asc = config::aac_specific_config(&mp4.tracks()[ai].configuration).unwrap();
    assert_eq!(
        NativeAacDecoder::new(asc).err().unwrap().to_string(),
        "AAC parametric stereo synthesis is not yet implemented"
    );
    let packets =
        include_bytes!("fixtures/playback-errors/he-aac-ps-matrix-controller-packets.bin");
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut controller = aac_ps_matrix_controller::State::default();
    let mut hybrid = aac_ps_hybrid::State::default();
    let mut state = decor::State::default();
    for (n, row) in case["frames"].as_array().unwrap().iter().enumerate() {
        let packet = &v["packet_frames"][n];
        let off = packet["offset"].as_u64().unwrap() as usize;
        let size = packet["bytes"].as_u64().unwrap() as usize;
        let mut data = vec![];
        mp4.read_packet(ai, n, &mut data).unwrap();
        assert_eq!(data, &packets[off..off + size]);
        let payload = hex(v["sbr_payloads"][n].as_str().unwrap());
        let mut bits = BitReader::new(&payload);
        let kind = bits.read(4).unwrap();
        assert_eq!(kind, if n == 1 { 14 } else { 13 });
        let frame = sbr
            .read(&mut bits, payload.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let parsed = ps
            .read_sbr_extensions(frame.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        assert_eq!(parsed.len(), 1);
        let m = controller.process(&parsed[0].parameters, 32).unwrap();
        assert_eq!(m.temporal.bands, bands(&row["bands"]));
        let mono = hybrid
            .process(m.temporal.bands, &qmf[n * 32..n * 32 + 32])
            .unwrap();
        let diffuse = state
            .process_frame(m.temporal.bands, controls(row), &mono.slots)
            .unwrap();
        check(&diffuse, &state, row);
        let stereo = m.mix(&mono.slots, &diffuse.output).unwrap();
        for c in 0..2 {
            compare(
                stereo.channels[c]
                    .iter()
                    .flatten()
                    .flat_map(|v| [v.re, v.im]),
                &row["stereo_hybrid"][c],
            );
            let qmf = aac_ps_hybrid::synthesize(m.temporal.bands, &stereo.channels[c]).unwrap();
            compare(
                qmf.iter().flatten().flat_map(|v| [v.re, v.im]),
                &row["stereo_qmf"][c],
            );
        }
    }
}
