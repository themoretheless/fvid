//! Stateful endpoint acceptance, distinct from pending full PS stereo PCM.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_history, aac_ps_mapping as common, aac_ps_phase_history as phase, aac_sbr_history,
    bits::BitReader,
};
use serde_json::Value;
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-phase-history-oracles.json"
    ))
    .unwrap()
}
fn native() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-history-oracles.json"
    ))
    .unwrap()
}
fn mixed() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-mapping-oracles.json"
    ))
    .unwrap()
}
fn bands(v: &Value) -> common::Bands {
    match v.as_u64().unwrap() {
        20 => common::Bands::Twenty,
        34 => common::Bands::ThirtyFour,
        _ => panic!(),
    }
}
fn ints(v: &Value) -> Vec<i16> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16)
        .collect()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn check_history(state: &phase::State, expected: &Value) {
    assert_eq!(state.bands(), bands(&expected["bands"]));
    for (a, key) in [(state.ipd(), "ipd"), (state.opd(), "opd")] {
        for (i, v) in a.iter().enumerate() {
            assert_eq!(*v, ints(&expected["history"][key][i]));
        }
    }
}
fn advance(state: &mut phase::State, parameters: &common::Frame, expected: &Value) {
    let before = state.clone();
    let result = state.process(parameters);
    if !expected["accepted"].as_bool().unwrap() {
        assert_eq!(
            result.unwrap_err().0,
            "PS phase history requires initialized parameters"
        );
        assert_eq!(*state, before);
        check_history(state, expected);
        return;
    }
    let result = result.unwrap();
    let mut replay = before.clone();
    assert_eq!(replay.process(parameters).unwrap(), result);
    assert_eq!(replay, *state);
    assert_eq!(
        result.bands_changed,
        expected["bands_changed"].as_bool().unwrap()
    );
    assert_eq!(result.bands, bands(&expected["bands"]));
    let ends = expected["endpoints"].as_array().unwrap();
    assert_eq!(result.endpoints.len(), ends.len());
    for (e, (actual, golden)) in result.endpoints.iter().zip(ends).enumerate() {
        assert_eq!(actual.len(), result.bands.count());
        for (b, actual) in actual.iter().enumerate() {
            for (i, a) in actual.coefficients().iter().enumerate() {
                let re = golden[b][i][0].as_str().unwrap().parse::<f64>().unwrap();
                let im = golden[b][i][1].as_str().unwrap().parse::<f64>().unwrap();
                assert!((a.re - re).abs() < 3e-11, "{e}/{b}/{i}: {} != {re}", a.re);
                assert!((a.im - im).abs() < 3e-11);
            }
        }
        for (k, binding) in result.bands.bindings().iter().enumerate() {
            let actual = result.hybrid_endpoint(e, k).unwrap();
            let b = usize::from(binding.parameter);
            for (i, a) in actual.coefficients().iter().enumerate() {
                let re = golden[b][i][0].as_str().unwrap().parse::<f64>().unwrap();
                let im = golden[b][i][1].as_str().unwrap().parse::<f64>().unwrap()
                    * if binding.conjugate { -1.0 } else { 1.0 };
                assert!((a.re - re).abs() < 3e-11);
                assert!((a.im - im).abs() < 3e-11);
            }
        }
    }
    assert!(result.hybrid_endpoint(result.endpoints.len(), 0).is_err());
    assert!(
        result
            .hybrid_endpoint(0, result.bands.bindings().len())
            .is_err()
    );
    if parameters.envelopes.is_empty() && parameters.bands == before.bands() {
        assert_eq!(*state, before)
    }
    check_history(state, expected);
}
#[test]
fn all_saved_native_sequences_preserve_two_positions_and_reset_only_on_grid_changes() {
    let g = reference();
    let n = native();
    let binary = include_bytes!("fixtures/playback-errors/aac-ps-history-syntax.bin");
    for (sequence, golden) in n["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .zip(g["sequences"].as_array().unwrap())
    {
        assert_eq!(sequence["name"], golden["name"]);
        let mut history = aac_ps_history::Stream::default();
        let mut mapping = common::State::default();
        let mut state = phase::State::default();
        let rows = sequence["frames"].as_array().unwrap();
        let expected = golden["frames"].as_array().unwrap();
        assert_eq!(rows.len(), expected.len());
        for (row, expected) in rows.iter().zip(expected) {
            let offset = row["offset"].as_u64().unwrap() as usize;
            let size = row["bytes"].as_u64().unwrap() as usize;
            let start = row["start"].as_u64().unwrap() as usize;
            let mut bits = BitReader::new(&binary[offset..offset + size]);
            bits.skip(start).unwrap();
            let parsed = history
                .read(
                    &mut bits,
                    start + row["bits"].as_u64().unwrap() as usize,
                    sequence["slots"].as_u64().unwrap() as u8,
                )
                .unwrap();
            let parameters = mapping.process(&parsed.parameters).unwrap();
            advance(&mut state, &parameters, expected);
        }
        state.reset();
        assert_eq!(state, phase::State::default());
    }
}
fn run_video(video: &Value, common_expected: &Value, expected: &Value, packets: &[u8]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(video["video"]["file"].as_str().unwrap());
    let mut reader =
        Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    assert!(reader.refused().is_empty());
    assert!(
        reader
            .tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && t.codec == *b"avc1")
    );
    let ai = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(
        (
            reader.tracks()[ai].sample_rate,
            reader.tracks()[ai].channels,
            reader.tracks()[ai].samples.len()
        ),
        (48000, 2, 3)
    );
    let asc = config::aac_specific_config(&reader.tracks()[ai].configuration).unwrap();
    assert_eq!(
        NativeAacDecoder::new(asc).err().unwrap().to_string(),
        "AAC parametric stereo synthesis is not yet implemented"
    );
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut mapping = common::State::default();
    let mut state = phase::State::default();
    for i in 0..3 {
        let row = &video["packet_frames"][i];
        let off = row["offset"].as_u64().unwrap() as usize;
        let length = row["bytes"].as_u64().unwrap() as usize;
        let mut data = vec![];
        reader.read_packet(ai, i, &mut data).unwrap();
        assert_eq!(data, &packets[off..off + length]);
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
        let parameters = mapping.process(&parsed[0].parameters).unwrap();
        assert_eq!(parameters.bands, bands(&common_expected[i]["bands"]));
        assert_eq!(
            parameters.phase_enabled,
            common_expected[i]["phase_enabled"].as_bool().unwrap()
        );
        assert_eq!(
            parameters.initialized,
            common_expected[i]["initialized"].as_bool().unwrap()
        );
        if parameters.initialized && parameters.envelopes.is_empty() {
            // A repeated packet needs its phase checkpoint/preroll. Fresh
            // physical state must not manufacture zero-valued history.
            let mut fresh = phase::State::default();
            let before = fresh.clone();
            assert_eq!(
                fresh.process(&parameters).unwrap_err().0,
                "PS phase history startup requires new envelopes"
            );
            assert_eq!(fresh, before);
        }
        advance(&mut state, &parameters, &expected[i]);
    }
}
#[test]
fn original_videos_cover_reuse_disable_grid_change_without_envelopes_and_startup_34() {
    let g = reference();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-phase-history-packets.bin");
    assert_eq!(g["videos"].as_array().unwrap().len(), 4);
    for video in g["videos"].as_array().unwrap() {
        run_video(video, &video["common"], &video["expected"], packets);
    }
}
#[test]
fn all_mixed_resolution_videos_route_smoothed_complex_matrices_to_starred_hybrid_bands() {
    let g = reference();
    let m = mixed();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-mapping-packets.bin");
    let videos = m["videos"].as_array().unwrap();
    let expected = g["mapped_videos"].as_array().unwrap();
    assert_eq!(videos.len(), expected.len());
    for (video, expected) in videos.iter().zip(expected) {
        assert_eq!(video["video"]["file"], expected["file"]);
        run_video(video, &video["expected"], &expected["frames"], packets);
    }
}
#[test]
fn late_envelope_errors_and_control_mismatches_do_not_commit_history() {
    let n = native();
    let row = &n["sequences"][0]["frames"][0];
    let binary = include_bytes!("fixtures/playback-errors/aac-ps-history-syntax.bin");
    let off = row["offset"].as_u64().unwrap() as usize;
    let size = row["bytes"].as_u64().unwrap() as usize;
    let start = row["start"].as_u64().unwrap() as usize;
    let mut bits = BitReader::new(&binary[off..off + size]);
    bits.skip(start).unwrap();
    let parsed = aac_ps_history::Stream::default()
        .read(
            &mut bits,
            start + row["bits"].as_u64().unwrap() as usize,
            24,
        )
        .unwrap();
    let mut mapping = common::State::default();
    let initial = mapping.process(&parsed.parameters).unwrap();
    let mut state = phase::State::default();
    let mut endpoints = state.process(&initial).unwrap();
    let mut valid = initial.clone();
    valid.previous_bands = state.bands();
    let checkpoint = state.clone();
    let mut bad = valid.clone();
    bad.envelopes[1].ipd[0] = 8;
    assert!(state.process(&bad).is_err());
    assert_eq!(state, checkpoint);
    let mut bad = valid.clone();
    bad.envelopes[1].iid[0] = i16::MAX;
    assert!(state.process(&bad).is_err());
    assert_eq!(state, checkpoint);
    let mut bad = valid.clone();
    bad.envelopes[1].ipd.pop();
    assert!(state.process(&bad).is_err());
    assert_eq!(state, checkpoint);
    let mut bad = valid.clone();
    bad.previous_bands = if state.bands() == common::Bands::Twenty {
        common::Bands::ThirtyFour
    } else {
        common::Bands::Twenty
    };
    assert!(state.process(&bad).is_err());
    assert_eq!(state, checkpoint);
    let mut bad = valid.clone();
    bad.initialized = false;
    assert!(state.process(&bad).is_err());
    assert_eq!(state, checkpoint);
    endpoints.endpoints[0][0].h11.re = f64::NAN;
    assert!(endpoints.hybrid_endpoint(0, 0).is_err());
    endpoints.endpoints[0].clear();
    assert!(endpoints.hybrid_endpoint(0, 0).is_err());
}
