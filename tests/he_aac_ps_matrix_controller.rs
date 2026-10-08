//! Matrix-controller/hybrid mixing acceptance, not full PS stereo PCM.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_history, aac_ps_mapping::Bands, aac_ps_matrix_controller as controller, aac_sbr_history,
    aac_sbr_qmf::Complex, bits::BitReader,
};
use serde_json::Value;
const COEFFICIENTS: &[u8] =
    include_bytes!("fixtures/playback-errors/aac-ps-matrix-controller-coefficients.bin");
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-matrix-controller-oracles.json"
    ))
    .unwrap()
}
fn native() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-history-oracles.json"
    ))
    .unwrap()
}
fn bits(row: &Value) -> BitReader<'static> {
    let bytes = include_bytes!("fixtures/playback-errors/aac-ps-history-syntax.bin");
    let off = row["offset"].as_u64().unwrap() as usize;
    let size = row["bytes"].as_u64().unwrap() as usize;
    let mut bits = BitReader::new(&bytes[off..off + size]);
    bits.skip(row["start"].as_u64().unwrap() as usize).unwrap();
    bits
}
fn bands(v: &Value) -> Bands {
    match v.as_u64().unwrap() {
        20 => Bands::Twenty,
        34 => Bands::ThirtyFour,
        _ => panic!(),
    }
}
fn compare(actual: impl IntoIterator<Item = f64>, descriptor: &Value) {
    let offset = descriptor[0].as_u64().unwrap() as usize;
    let count = descriptor[1].as_u64().unwrap() as usize;
    let bytes = &COEFFICIENTS[offset..offset + count * 8];
    let actual: Vec<_> = actual.into_iter().collect();
    assert_eq!(actual.len(), count);
    for (a, e) in actual.into_iter().zip(bytes.chunks_exact(8)) {
        let e = f64::from_le_bytes(e.try_into().unwrap());
        assert!(a.is_finite() && e.is_finite());
        assert!((a - e).abs() < 6e-11, "{a} != {e}");
    }
}
fn signals(slots: usize, bands: Bands) -> (Vec<Vec<Complex>>, Vec<Vec<Complex>>) {
    let count = bands.bindings().len();
    let source = (0..slots)
        .map(|n| {
            (0..count)
                .map(|k| Complex {
                    re: ((13 * n + 7 * k) % 23) as f64 / 16.0 - 11.0 / 16.0,
                    im: ((3 * n + 11 * k) % 31) as f64 / 32.0 - 15.0 / 32.0,
                })
                .collect()
        })
        .collect();
    let diffuse = (0..slots)
        .map(|n| {
            (0..count)
                .map(|k| Complex {
                    re: ((17 * n + 5 * k) % 29) as f64 / 32.0 - 14.0 / 32.0,
                    im: ((19 * n + 3 * k) % 37) as f64 / 16.0 - 18.0 / 16.0,
                })
                .collect()
        })
        .collect();
    (source, diffuse)
}
fn snapshot(state: &controller::State, e: &Value) {
    assert_eq!(state.bands(), bands(&e["bands"]));
    compare(
        state.retained_real().iter().flat_map(|m| m.coefficients()),
        &e["real"],
    );
    compare(
        state
            .retained_complex()
            .iter()
            .flat_map(|m| m.coefficients().into_iter().flat_map(|c| [c.re, c.im])),
        &e["last"],
    );
    assert_eq!(state.phase().bands(), state.bands());
    for (a, key) in [(state.phase().ipd(), "ipd"), (state.phase().opd(), "opd")] {
        for (i, a) in a.iter().enumerate() {
            assert_eq!(
                *a,
                e["phase"][key][i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap() as i16)
                    .collect::<Vec<_>>()
            );
        }
    }
}
fn advance(state: &mut controller::State, p: &aac_ps_history::Parameters, e: &Value) {
    let before = state.clone();
    let slots = e["slots"].as_u64().unwrap() as u8;
    let result = state.process(p, slots);
    if !e["accepted"].as_bool().unwrap() {
        assert_eq!(
            result.unwrap_err().0,
            "PS matrix controller requires initialized parameters"
        );
        assert_eq!(*state, before);
        snapshot(state, e);
        return;
    }
    let result = result.unwrap();
    assert_eq!(result.bands_changed, e["bands_changed"].as_bool().unwrap());
    assert_eq!(result.temporal.bands, bands(&e["bands"]));
    let rows = e["coefficients"].as_array().unwrap();
    assert_eq!(result.temporal.coefficients.len(), rows.len());
    for (a, e) in result.temporal.coefficients.iter().zip(rows) {
        compare(
            a.iter()
                .flat_map(|m| m.coefficients().into_iter().flat_map(|c| [c.re, c.im])),
            e,
        );
    }
    snapshot(state, e);
    let mut replay = before.clone();
    assert_eq!(replay.process(p, slots).unwrap(), result);
    assert_eq!(replay, *state);
    if let Some(output) = e.get("output") {
        let (mono, diffuse) = signals(usize::from(slots), result.temporal.bands);
        let actual = result.mix(&mono, &diffuse).unwrap();
        let mut atomic = before.clone();
        assert_eq!(
            atomic.process_and_mix(p, slots, &mono, &diffuse).unwrap(),
            actual
        );
        assert_eq!(atomic, *state);
        assert_eq!(actual.bands, result.temporal.bands);
        assert_eq!(actual.bands_changed, result.bands_changed);
        for c in 0..2 {
            assert_eq!(actual.channels[c].len(), usize::from(slots));
            for (a, e) in actual.channels[c].iter().zip(output[c].as_array().unwrap()) {
                compare(a.iter().flat_map(|c| [c.re, c.im]), e);
            }
        }
        // Late audio failure must undo ALL coupled matrix/phase/grid state.
        let mut invalid = mono.clone();
        invalid.last_mut().unwrap().last_mut().unwrap().im = f64::NAN;
        let mut atomic = before.clone();
        assert!(
            atomic
                .process_and_mix(p, slots, &invalid, &diffuse)
                .is_err()
        );
        assert_eq!(atomic, before);
        assert_eq!(
            atomic.process_and_mix(p, slots, &mono, &diffuse).unwrap(),
            actual
        );
        assert_eq!(atomic, *state);
    }
    assert!(result.hybrid_matrix(usize::from(slots), 0).is_err());
    assert!(
        result
            .hybrid_matrix(0, result.temporal.bands.bindings().len())
            .is_err()
    );
}
#[test]
fn all_native_sequences_match_complete_slot_grids_and_retained_state() {
    let g = reference();
    let n = native();
    let sequences = n["sequences"].as_array().unwrap();
    let expected = g["sequences"].as_array().unwrap();
    assert_eq!(sequences.len(), expected.len());
    for (s, e) in sequences.iter().zip(expected) {
        assert_eq!(s["name"], e["name"]);
        let mut history = aac_ps_history::Stream::default();
        let mut state = controller::State::default();
        let frames = s["frames"].as_array().unwrap();
        let golden = e["frames"].as_array().unwrap();
        assert_eq!(frames.len(), golden.len());
        for (row, e) in frames.iter().zip(golden) {
            let end =
                row["start"].as_u64().unwrap() as usize + row["bits"].as_u64().unwrap() as usize;
            let parsed = history
                .read(&mut bits(row), end, s["slots"].as_u64().unwrap() as u8)
                .unwrap();
            advance(&mut state, &parsed.parameters, e);
        }
        state.reset();
        assert_eq!(state, controller::State::default());
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn video(video: &Value, expected: &Value, packets: &[u8]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(video["video"]["file"].as_str().unwrap());
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    assert!(mp4.refused().is_empty());
    assert!(
        mp4.tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && t.codec == *b"avc1")
    );
    let ai = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(
        (
            mp4.tracks()[ai].sample_rate,
            mp4.tracks()[ai].channels,
            mp4.tracks()[ai].samples.len()
        ),
        (48000, 2, 3)
    );
    let asc = config::aac_specific_config(&mp4.tracks()[ai].configuration).unwrap();
    assert_eq!(
        NativeAacDecoder::new(asc).err().unwrap().to_string(),
        "AAC parametric stereo synthesis is not yet implemented"
    );
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut state = controller::State::default();
    for i in 0..3 {
        let row = &video["packet_frames"][i];
        let off = row["offset"].as_u64().unwrap() as usize;
        let size = row["bytes"].as_u64().unwrap() as usize;
        let mut data = vec![];
        mp4.read_packet(ai, i, &mut data).unwrap();
        assert_eq!(data, &packets[off..off + size]);
        let raw = hex(video["sbr_payloads"][i].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        assert_eq!(kind, if i == 1 { 14 } else { 13 });
        let f = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let parsed = ps
            .read_sbr_extensions(f.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0].parameters;
        if p.initialized && p.envelopes.is_empty() {
            let mut fresh = controller::State::default();
            let before = fresh.clone();
            assert_eq!(
                fresh.process(p, 32).unwrap_err().0,
                "PS phase history startup requires new envelopes"
            );
            assert_eq!(fresh, before);
        }
        advance(&mut state, p, &expected[i]);
    }
}
#[test]
fn new_retained_phase_and_real_grid_transition_videos_accept_the_complete_matrix_path() {
    let g = reference();
    let packets =
        include_bytes!("fixtures/playback-errors/he-aac-ps-matrix-controller-packets.bin");
    let videos = g["videos"].as_array().unwrap();
    assert_eq!(videos.len(), 2);
    for v in videos {
        video(v, &v["expected"], packets);
    }
}
#[test]
fn all_previous_mixed_and_phase_videos_match_independent_hybrid_stereo_outputs() {
    let g = reference();
    let mapping: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-mapping-oracles.json"
    ))
    .unwrap();
    let phase: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-phase-history-oracles.json"
    ))
    .unwrap();
    let frames = g["existing_videos"].as_array().unwrap();
    assert_eq!(frames.len(), 11);
    for frame in frames {
        let (source, packets): (&Value, &[u8]) = if frame["source"] == "aac-ps-mapping-oracles.json"
        {
            (
                &mapping,
                include_bytes!("fixtures/playback-errors/he-aac-ps-mapping-packets.bin"),
            )
        } else {
            (
                &phase,
                include_bytes!("fixtures/playback-errors/he-aac-ps-phase-history-packets.bin"),
            )
        };
        let v = source["videos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["video"]["file"] == frame["file"])
            .unwrap();
        video(v, &frame["frames"], packets);
    }
}
#[test]
fn malformed_controls_late_signal_failure_and_slot_epoch_are_transactional() {
    let n = native();
    let row = &n["sequences"][0]["frames"][0];
    let end = row["start"].as_u64().unwrap() as usize + row["bits"].as_u64().unwrap() as usize;
    let parsed = aac_ps_history::Stream::default()
        .read(&mut bits(row), end, 24)
        .unwrap();
    let p = parsed.parameters;
    let mut state = controller::State::default();
    state.process(&p, 24).unwrap();
    let before = state.clone();
    assert_eq!(
        state.process(&p, 32).unwrap_err().0,
        "PS matrix controller slot count changed without reset"
    );
    assert_eq!(state, before);
    let mut bad = p.clone();
    bad.borders[1] = 24;
    assert!(state.process(&bad, 24).is_err());
    assert_eq!(state, before);
    let mut bad = p.clone();
    bad.envelopes[1].iid[0] = i16::MAX;
    assert!(state.process(&bad, 24).is_err());
    assert_eq!(state, before);
    let (mono, diffuse) = signals(24, state.bands());
    let mut bad = mono.clone();
    bad[23].pop();
    assert!(state.process_and_mix(&p, 24, &bad, &diffuse).is_err());
    assert_eq!(state, before);
    let mut bad = mono.clone();
    bad[23].last_mut().unwrap().im = f64::INFINITY;
    assert!(state.process_and_mix(&p, 24, &bad, &diffuse).is_err());
    assert_eq!(state, before);
    let mut frame = before.clone().process(&p, 24).unwrap();
    frame.temporal.coefficients[0].clear();
    assert!(frame.mix(&mono, &diffuse).is_err());
    let mut frame = before.clone().process(&p, 24).unwrap();
    frame.temporal.coefficients[23][0].h11.re = f64::NAN;
    assert!(frame.mix(&mono, &diffuse).is_err());
    state.reset();
    state.process(&p, 32).unwrap();
    assert_eq!(state.bands(), before.bands());
}
