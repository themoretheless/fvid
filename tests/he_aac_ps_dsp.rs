//! Full aligned QMF-to-PCM acceptance; native encoded PS remains separate.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_decorrelation::FrameControls,
    aac_ps_dsp::{Dsp, LOOKAHEAD},
    aac_ps_history::{self, Parameters},
    aac_ps_mapping::Bands,
    aac_sbr_dsp::OutputRate,
    aac_sbr_history,
    aac_sbr_qmf::Complex,
    bits::BitReader,
};
use serde_json::Value;
const DATA: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap()
}
fn scalars(v: &Value) -> Vec<f64> {
    let off = v[0].as_u64().unwrap() as usize;
    let count = v[1].as_u64().unwrap() as usize;
    DATA[off..off + count * 8]
        .chunks_exact(8)
        .map(|r| f64::from_le_bytes(r.try_into().unwrap()))
        .collect()
}
fn input(case: &Value) -> Vec<[Complex; 64]> {
    scalars(&case["input"])
        .chunks_exact(128)
        .map(|r| {
            std::array::from_fn(|k| Complex {
                re: r[2 * k],
                im: r[2 * k + 1],
            })
        })
        .collect()
}
fn compare(actual: impl IntoIterator<Item = f64>, expected: &Value) {
    let a: Vec<_> = actual.into_iter().collect();
    let e = scalars(expected);
    assert_eq!(a.len(), e.len());
    for (a, e) in a.into_iter().zip(e) {
        assert!(a.is_finite());
        assert!((a - e).abs() < 4e-11 * (1. + e.abs()), "{a} != {e}");
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|r| u8::from_str_radix(std::str::from_utf8(r).unwrap(), 16).unwrap())
        .collect()
}
fn parameters(case: &Value) -> Vec<Parameters> {
    let source = &case["source"];
    let name = source["name"].as_str().unwrap();
    let mut ps = aac_ps_history::Stream::default();
    if source["kind"] == "sequence" {
        let h: Value = serde_json::from_str(include_str!(
            "fixtures/playback-errors/aac-ps-history-oracles.json"
        ))
        .unwrap();
        let seq = h["sequences"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap();
        let slots = seq["slots"].as_u64().unwrap() as u8;
        let data = include_bytes!("fixtures/playback-errors/aac-ps-history-syntax.bin");
        return seq["frames"].as_array().unwrap()[..3]
            .iter()
            .map(|row| {
                let off = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let mut bits = BitReader::new(&data[off..off + len]);
                let start = row["start"].as_u64().unwrap() as usize;
                bits.skip(start).unwrap();
                ps.read(
                    &mut bits,
                    start + row["bits"].as_u64().unwrap() as usize,
                    slots,
                )
                .unwrap()
                .parameters
            })
            .collect();
    }
    let m: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-matrix-controller-oracles.json"
    ))
    .unwrap();
    let v = m["videos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["video"]["file"] == name)
        .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name);
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let ai = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(mp4.tracks()[ai].samples.len(), 3);
    let asc = config::aac_specific_config(&mp4.tracks()[ai].configuration).unwrap();
    assert_eq!(
        NativeAacDecoder::new(asc).err().unwrap().to_string(),
        "AAC parametric stereo synthesis is not yet implemented"
    );
    let packets =
        include_bytes!("fixtures/playback-errors/he-aac-ps-matrix-controller-packets.bin");
    let mut sbr = aac_sbr_history::Stream::default();
    (0..3)
        .map(|n| {
            let row = &v["packet_frames"][n];
            let off = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let mut data = vec![];
            mp4.read_packet(ai, n, &mut data).unwrap();
            assert_eq!(data, &packets[off..off + len]);
            let raw = hex(v["sbr_payloads"][n].as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            assert_eq!(kind, if n == 1 { 14 } else { 13 });
            let f = sbr
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
                .unwrap();
            let mut parsed = ps
                .read_sbr_extensions(f.syntax.data.extended_data.as_ref().unwrap(), 32)
                .unwrap();
            assert_eq!(parsed.len(), 1);
            parsed.remove(0).parameters
        })
        .collect()
}
#[test]
fn every_left_right_pcm_and_qmf_sample_matches_the_aligned_independent_reference() {
    let refs = reference();
    let cases = refs["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    assert_eq!(LOOKAHEAD, 6);
    for case in cases {
        let params = parameters(case);
        let qmf = input(case);
        if case["zero_eof"] == true {
            assert!(
                qmf[qmf.len() - 6..]
                    .iter()
                    .flatten()
                    .all(|&c| c == Complex::default())
            );
        }
        for rate in [OutputRate::Double, OutputRate::Core] {
            let mut state = Dsp::default();
            for (row, parameters) in case["frames"].as_array().unwrap().iter().zip(&params) {
                let slots = row["slots"].as_u64().unwrap() as u8;
                let start = row["input_start"].as_u64().unwrap() as usize;
                let block = &qmf[start..start + usize::from(slots) + LOOKAHEAD];
                let result = state
                    .process(parameters, slots, block, FrameControls::default(), rate)
                    .unwrap();
                assert_eq!(result.slots, slots);
                assert_eq!(result.output_rate, rate);
                assert_eq!(state.output_rate(), Some(rate));
                assert_eq!(
                    result.bands,
                    if row["bands"] == 20 {
                        Bands::Twenty
                    } else {
                        Bands::ThirtyFour
                    }
                );
                assert_eq!(
                    state.retained_lookahead().unwrap().as_slice(),
                    &block[usize::from(slots)..]
                );
                let width = if rate == OutputRate::Double { 64 } else { 32 };
                let key = if rate == OutputRate::Double {
                    "Double"
                } else {
                    "Core"
                };
                for c in 0..2 {
                    assert_eq!(result.pcm[c].len(), usize::from(slots) * width);
                    compare(result.pcm[c].iter().copied(), &row[key][c]);
                    compare(
                        result.qmf[c].iter().flatten().flat_map(|c| [c.re, c.im]),
                        &row["qmf"][c],
                    );
                }
                assert_ne!(
                    result.pcm[0], result.pcm[1],
                    "must reconstruct actual stereo, not duplicate mono"
                );
            }
        }
    }
}
#[test]
fn checkpoint_replay_and_reset_include_both_synthesis_histories_and_overlap() {
    for case in reference()["cases"].as_array().unwrap() {
        let params = parameters(case);
        let qmf = input(case);
        for rate in [OutputRate::Double, OutputRate::Core] {
            let mut state = Dsp::default();
            let mut results = vec![];
            for (row, p) in case["frames"].as_array().unwrap().iter().zip(&params) {
                let slots = row["slots"].as_u64().unwrap() as u8;
                let start = row["input_start"].as_u64().unwrap() as usize;
                let block = &qmf[start..start + usize::from(slots) + LOOKAHEAD];
                let before = state.clone();
                let mut replay = before;
                let out = state
                    .process(p, slots, block, FrameControls::default(), rate)
                    .unwrap();
                assert_eq!(
                    out,
                    replay
                        .process(p, slots, block, FrameControls::default(), rate)
                        .unwrap()
                );
                assert_eq!(state, replay);
                results.push(out);
            }
            state.reset();
            assert_eq!(state, Dsp::default());
            assert_eq!(state.output_rate(), None);
            assert!(state.retained_lookahead().is_none());
            for ((row, p), expected) in case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&params)
                .zip(results)
            {
                let slots = row["slots"].as_u64().unwrap() as u8;
                let start = row["input_start"].as_u64().unwrap() as usize;
                assert_eq!(
                    state
                        .process(
                            p,
                            slots,
                            &qmf[start..start + usize::from(slots) + LOOKAHEAD],
                            FrameControls::default(),
                            rate
                        )
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
#[test]
fn late_failures_roll_back_matrix_hybrid_decorrelation_pcm_and_retained_lookahead_together() {
    let refs = reference();
    let case = &refs["cases"][2];
    let params = parameters(case);
    let qmf = input(case);
    let mut state = Dsp::default();
    state
        .process(
            &params[0],
            32,
            &qmf[..38],
            FrameControls::default(),
            OutputRate::Double,
        )
        .unwrap();
    let before = state.clone();
    let block = &qmf[32..70];
    // This fails after the matrix and hybrid owners advanced on the trial.
    assert!(
        state
            .process(
                &params[1],
                32,
                block,
                FrameControls {
                    previous_ps_present: false,
                    qmf_limit: 65
                },
                OutputRate::Double
            )
            .is_err()
    );
    assert_eq!(state, before);
    let mut huge = block.to_vec();
    huge[37][0].re = 1e157;
    let err = state
        .process(
            &params[1],
            32,
            &huge,
            FrameControls::default(),
            OutputRate::Double,
        )
        .unwrap_err();
    assert!(err.0.contains("energy is not representable"), "{err}");
    assert_eq!(state, before);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut malformed = block.to_vec();
        malformed[37][63].im = bad;
        assert!(
            state
                .process(
                    &params[1],
                    32,
                    &malformed,
                    FrameControls::default(),
                    OutputRate::Double
                )
                .is_err()
        );
        assert_eq!(state, before);
    }
    assert!(
        state
            .process(
                &params[1],
                32,
                block,
                FrameControls::default(),
                OutputRate::Core
            )
            .is_err()
    );
    assert_eq!(state, before);
    for slots in [0, 23, 25, 31, 33] {
        assert!(
            state
                .process(
                    &params[1],
                    slots,
                    block,
                    FrameControls::default(),
                    OutputRate::Double
                )
                .is_err()
        );
        assert_eq!(state, before);
    }
    assert!(
        state
            .process(
                &params[1],
                32,
                &block[..32],
                FrameControls::default(),
                OutputRate::Double
            )
            .is_err()
    );
    assert_eq!(state, before);
    assert!(
        state
            .process(
                &params[1],
                30,
                &block[..36],
                FrameControls::default(),
                OutputRate::Double
            )
            .is_err()
    );
    assert_eq!(state, before);
    let mut wrong = block.to_vec();
    wrong[0][0].re += 0.125;
    let err = state
        .process(
            &params[1],
            32,
            &wrong,
            FrameControls::default(),
            OutputRate::Double,
        )
        .unwrap_err();
    assert!(err.0.contains("overlap"));
    assert_eq!(state, before);
    assert!(
        state
            .process(
                &params[1],
                32,
                block,
                FrameControls::default(),
                OutputRate::Double
            )
            .is_ok()
    );
}
