//! Numeric mapping acceptance; full PS PCM synthesis remains a separate gate.
use fvid::container::mp4::{Limits, Mp4Reader};
use fvid_media::owned_aac::{
    aac_ps_data::{IccMode, IidMode},
    aac_ps_history, aac_ps_mapping as m, aac_sbr_history,
    bits::BitReader,
};
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-mapping-oracles.json"
    ))
    .unwrap()
}
fn bands(v: &Value) -> m::Bands {
    match v.as_u64().unwrap() {
        20 => m::Bands::Twenty,
        34 => m::Bands::ThirtyFour,
        _ => panic!(),
    }
}
fn integers(v: &Value) -> Vec<i16> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16)
        .collect()
}
fn real(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v.as_str()
                .map(|s| s.parse().unwrap())
                .unwrap_or_else(|| v.as_f64().unwrap())
        })
        .collect()
}
fn close(actual: &[f64], expected: &Value) {
    let expected = real(expected);
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() <= 2e-14 * b.abs().max(1.0), "{a} != {b}");
    }
}
fn check(actual: &m::Indices, expected: &Value) {
    assert_eq!(actual.bands, bands(&expected["bands"]));
    for (a, k) in [
        (&actual.iid, "iid"),
        (&actual.icc, "icc"),
        (&actual.ipd, "ipd"),
        (&actual.opd, "opd"),
    ] {
        assert_eq!(*a, integers(&expected[k]), "{k}");
    }
    let l = actual.dequantize().unwrap();
    for (a, k) in [
        (&l.iid_db, "iid_db"),
        (&l.coherence, "coherence"),
        (&l.ipd_radians, "ipd_radians"),
        (&l.opd_radians, "opd_radians"),
    ] {
        close(a, &expected["levels"][k]);
    }
}
#[test]
fn independent_integer_phase_and_real_oracles() {
    let g = manifest();
    for k in ["integers", "phases"] {
        for c in g[k].as_array().unwrap() {
            let src = integers(&c["source"]);
            let dst = bands(&c["bands"]);
            let a = if k == "integers" {
                m::map_indices(&src, dst)
            } else {
                m::map_phase(&src, dst)
            }
            .unwrap();
            assert_eq!(a, integers(&c["expected"]));
        }
    }
    for c in g["coefficients"].as_array().unwrap() {
        close(
            &m::map_coefficients(&real(&c["source"]), bands(&c["bands"])).unwrap(),
            &c["expected"],
        );
    }
}
#[test]
fn every_native_resolution_quantizer_and_mixing_mode() {
    for c in manifest()["native"].as_array().unwrap() {
        let s = &c["source"];
        let n = aac_ps_history::Indices {
            iid_mode: IidMode::new(s["iid_mode"].as_u64().unwrap() as u8).unwrap(),
            icc_mode: IccMode::new(s["icc_mode"].as_u64().unwrap() as u8).unwrap(),
            phase_mode: IidMode::new(s["phase_mode"].as_u64().unwrap() as u8).unwrap(),
            iid_enabled: s["iid_enabled"].as_bool().unwrap(),
            icc_enabled: s["icc_enabled"].as_bool().unwrap(),
            phase_enabled: s["phase_enabled"].as_bool().unwrap(),
            iid: integers(&s["iid"]),
            icc: integers(&s["icc"]),
            ipd: integers(&s["ipd"]),
            opd: integers(&s["opd"]),
        };
        check(
            &m::map_parameters(&n, bands(&c["expected"]["bands"])).unwrap(),
            &c["expected"],
        );
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn synthetic_videos_accept_mixed_resolutions_and_disabled_selection() {
    let g = manifest();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-mapping-packets.bin");
    for video in g["videos"].as_array().unwrap() {
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
        let mut sbr = aac_sbr_history::Stream::default();
        let mut ps = aac_ps_history::Stream::default();
        let mut map = m::State::default();
        for i in 0..3 {
            let row = &video["packet_frames"][i];
            let off = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let mut packet = vec![];
            reader.read_packet(ai, i, &mut packet).unwrap();
            assert_eq!(packet, &packets[off..off + len]);
            let raw = hex(video["sbr_payloads"][i].as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            let f = sbr
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
                .unwrap();
            let parsed = ps
                .read_sbr_extensions(f.syntax.data.extended_data.as_ref().unwrap(), 32)
                .unwrap();
            assert_eq!(parsed.len(), 1);
            let actual = map.process(&parsed[0].parameters).unwrap();
            let e = &video["expected"][i];
            assert_eq!(actual.bands, bands(&e["bands"]));
            assert_eq!(actual.previous_bands, bands(&e["previous_bands"]));
            assert_eq!(
                actual.borders,
                e["borders"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>()
            );
            let rows = e["envelopes"].as_array().unwrap();
            assert_eq!(actual.envelopes.len(), rows.len());
            for (a, e) in actual.envelopes.iter().zip(rows) {
                check(a, e);
            }
        }
    }
}
#[test]
fn malformed_sources_and_negative_rounding() {
    for size in [0, 1, 9, 11, 19, 21, 33, 35] {
        assert!(m::map_indices(&vec![0; size], m::Bands::Twenty).is_err());
    }
    assert!(m::map_phase(&[8; 5], m::Bands::Twenty).is_err());
    assert!(m::map_coefficients(&[f64::NAN; 20], m::Bands::ThirtyFour).is_err());
    assert!(m::map_coefficients(&[f64::INFINITY; 34], m::Bands::Twenty).is_err());
    let mut v = vec![0; 20];
    v[0] = -1;
    v[1] = -2;
    assert_eq!(m::map_indices(&v, m::Bands::ThirtyFour).unwrap()[1], -1);
    let mut h = vec![0.; 20];
    h[0] = -0.25;
    h[1] = -0.5;
    assert_eq!(
        m::map_coefficients(&h, m::Bands::ThirtyFour).unwrap()[1],
        -0.375
    );
    for b in [m::Bands::Twenty, m::Bands::ThirtyFour] {
        assert!(b.binding(b.bindings().len()).is_err());
        for x in b.bindings() {
            assert!(x.qmf < 64);
            assert!((x.parameter as usize) < b.count());
        }
    }
}

#[test]
fn history_sequences_checkpoint_rollback_and_no_envelope_transitions() {
    let native: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-history-oracles.json"
    ))
    .unwrap();
    let binary = include_bytes!("fixtures/playback-errors/aac-ps-history-syntax.bin");
    let golden = manifest();
    for (seq, expected) in native["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .zip(golden["sequences"].as_array().unwrap())
    {
        assert_eq!(seq["name"], expected["name"]);
        let mut history = aac_ps_history::Stream::default();
        let mut state = m::State::default();
        for (row, e) in seq["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(expected["frames"].as_array().unwrap())
        {
            let offset = row["offset"].as_u64().unwrap() as usize;
            let size = row["bytes"].as_u64().unwrap() as usize;
            let start = row["start"].as_u64().unwrap() as usize;
            let mut bits = BitReader::new(&binary[offset..offset + size]);
            bits.skip(start).unwrap();
            let p = history
                .read(
                    &mut bits,
                    start + row["bits"].as_u64().unwrap() as usize,
                    seq["slots"].as_u64().unwrap() as u8,
                )
                .unwrap()
                .parameters;
            let before = state.clone();
            let mut bad = p.clone();
            bad.borders.push(32);
            assert!(state.process(&bad).is_err());
            assert_eq!(state, before);
            if !p.envelopes.is_empty() {
                let mut bad = p.clone();
                bad.envelopes[0].iid[0] = i16::MAX;
                assert!(state.process(&bad).is_err());
                assert_eq!(state, before);
            }
            let result = state.process(&p).unwrap();
            assert_eq!(before.clone().process(&p).unwrap(), result);
            assert_eq!(result.bands, bands(&e["bands"]));
            assert_eq!(result.previous_bands, bands(&e["previous_bands"]));
            assert_eq!(
                result.bands_changed(),
                result.previous_bands != result.bands
            );
            let rows = e["envelopes"].as_array().unwrap();
            assert_eq!(result.envelopes.len(), rows.len());
            for (a, e) in result.envelopes.iter().zip(rows) {
                check(a, e);
                let mut bad = a.clone();
                bad.ipd.pop();
                assert!(bad.validate().is_err());
                let mut bad = a.clone();
                bad.ipd[a.bands.phase_bands()] = 1;
                assert!(bad.validate().is_err());
            }
        }
        state.reset();
        assert_eq!(state, m::State::default());
    }
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut state = m::State::default();
    for (i, payload) in native["mode_transition"]["sbr_payloads"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let raw = hex(payload.as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        let frame = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let parsed = ps
            .read_sbr_extensions(frame.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        let mapped = state.process(&parsed[0].parameters).unwrap();
        assert_eq!(
            mapped.bands,
            if i == 0 {
                m::Bands::Twenty
            } else {
                m::Bands::ThirtyFour
            }
        );
        if i > 0 {
            assert!(mapped.envelopes.is_empty());
            assert_eq!(parsed[0].parameters.retained.iid.len(), 10);
        }
    }
}

#[test]
fn hybrid_routing_matches_numeric_protocol_and_finite_extrema_stay_finite() {
    let protocol: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-mapping-protocol.json"
    ))
    .unwrap();
    for (b, key, count) in [
        (m::Bands::Twenty, "hybrid20", 71),
        (m::Bands::ThirtyFour, "hybrid34", 91),
    ] {
        let rows = protocol[key].as_array().unwrap();
        assert_eq!(b.bindings().len(), count);
        assert_eq!(rows.len(), count);
        let mut covered = [false; 64];
        for (i, row) in rows.iter().enumerate() {
            let x = b.binding(i).unwrap();
            assert_eq!(u64::from(x.qmf), row["qmf"].as_u64().unwrap());
            assert_eq!(u64::from(x.parameter), row["parameter"].as_u64().unwrap());
            assert_eq!(x.conjugate, row["conjugate"].as_bool().unwrap());
            covered[x.qmf as usize] = true;
        }
        assert!(covered.iter().all(|v| *v));
        for size in [20, 34] {
            for value in [f64::MAX, -f64::MAX] {
                assert!(
                    m::map_coefficients(&vec![value; size], b)
                        .unwrap()
                        .iter()
                        .all(|v| v.is_finite())
                );
            }
        }
    }
}
