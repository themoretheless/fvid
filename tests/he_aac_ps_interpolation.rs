//! Temporal matrix acceptance; full PS stereo PCM synthesis remains pending.
use fvid::container::mp4::{Limits, Mp4Reader};
use fvid_media::owned_aac::{
    aac_ps_history, aac_ps_interpolation as temporal, aac_ps_mapping as common,
    aac_ps_mixing as mixing, aac_sbr_history, aac_sbr_qmf::Complex, bits::BitReader,
};
use serde_json::Value;
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-interpolation-oracles.json"
    ))
    .unwrap()
}
fn bands(n: u64) -> common::Bands {
    match n {
        20 => common::Bands::Twenty,
        34 => common::Bands::ThirtyFour,
        _ => panic!(),
    }
}
fn border(v: &Value) -> Vec<u8> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect()
}
fn scalar(v: &Value) -> f64 {
    let s = v.as_str().unwrap();
    if let Some((a, b)) = s.split_once('/') {
        a.parse::<f64>().unwrap() / b.parse::<f64>().unwrap()
    } else {
        s.parse().unwrap()
    }
}
fn matrices(v: &Value) -> Vec<mixing::ComplexMatrix> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let c = |i: usize| Complex {
                re: scalar(&row[i][0]),
                im: scalar(&row[i][1]),
            };
            mixing::ComplexMatrix {
                h11: c(0),
                h12: c(1),
                h21: c(2),
                h22: c(3),
            }
        })
        .collect()
}
fn check(a: &[mixing::ComplexMatrix], e: &Value) {
    let expected = matrices(e);
    assert_eq!(a.len(), expected.len());
    for (a, e) in a.iter().zip(expected) {
        for (a, e) in a.coefficients().iter().zip(e.coefficients()) {
            assert!((a.re - e.re).abs() < 3e-11, "{} != {}", a.re, e.re);
            assert!((a.im - e.im).abs() < 3e-11);
        }
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn independent_piecewise_oracles_cover_startup_zero_borders_interior_tail_and_reuse() {
    for case in reference()["cases"].as_array().unwrap() {
        let b = bands(case["bands"].as_u64().unwrap());
        let mut state = temporal::State::new(b);
        let previous = matrices(&case["previous"]);
        state.replace_previous(b, &previous).unwrap();
        let endpoints: Vec<_> = case["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(matrices)
            .collect();
        let result = state
            .process(
                case["slots"].as_u64().unwrap() as u8,
                &border(&case["borders"]),
                &endpoints,
            )
            .unwrap();
        assert_eq!(result.bands, b);
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(result.coefficients.len(), expected.len());
        for (a, e) in result.coefficients.iter().zip(expected) {
            check(a, e);
        }
        assert_eq!(state.previous(), endpoints.last().unwrap_or(&previous));
        let checkpoint = state.clone();
        assert!(state.process(32, &[32], &[previous.clone()]).is_err());
        assert_eq!(state, checkpoint);
        let repeated = state.process(32, &[], &[]).unwrap();
        assert!(
            repeated
                .coefficients
                .iter()
                .all(|v| v == checkpoint.previous())
        );
        state.reset(b);
        assert!(
            state
                .previous()
                .iter()
                .all(|v| *v == mixing::ComplexMatrix::default())
        );
    }
}
#[test]
fn original_videos_accept_zero_first_border_short_tail_and_reuse_through_matrix_stage() {
    let g = reference();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-interpolation-packets.bin");
    for video in g["videos"].as_array().unwrap() {
        let b = bands(video["bands"].as_u64().unwrap());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(video["video"]["file"].as_str().unwrap());
        let mut mp4 =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
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
        // Keep full playback refusal distinct from this numeric-stage
        // acceptance until the hybrid/decorrelator/stereo synthesis is wired.
        let asc =
            fvid::codec::config::aac_specific_config(&mp4.tracks()[ai].configuration).unwrap();
        assert_eq!(
            fvid::codec::aac_native::NativeAacDecoder::new(asc)
                .err()
                .unwrap()
                .to_string(),
            "AAC parametric stereo synthesis is not yet implemented"
        );
        let mut sbr = aac_sbr_history::Stream::default();
        let mut history = aac_ps_history::Stream::default();
        let mut mapping = common::State::default();
        let mut temporal = temporal::State::new(b);
        for i in 0..3 {
            let record = &video["packet_frames"][i];
            let offset = record["offset"].as_u64().unwrap() as usize;
            let len = record["bytes"].as_u64().unwrap() as usize;
            let mut data = vec![];
            mp4.read_packet(ai, i, &mut data).unwrap();
            assert_eq!(data, &packets[offset..offset + len]);
            let raw = hex(video["sbr_payloads"][i].as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            let f = sbr
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
                .unwrap();
            let parsed = history
                .read_sbr_extensions(f.syntax.data.extended_data.as_ref().unwrap(), 32)
                .unwrap();
            assert_eq!(parsed.len(), 1);
            let common = mapping.process(&parsed[0].parameters).unwrap();
            assert_eq!(common.bands, b);
            assert!(!common.phase_enabled);
            let endpoints: Vec<Vec<_>> = common
                .envelopes
                .iter()
                .map(|p| {
                    mixing::envelope(p)
                        .unwrap()
                        .into_iter()
                        .map(|m| mixing::phase_matrix(m, [0; 3], [0; 3], false).unwrap())
                        .collect()
                })
                .collect();
            let expected = &video["expected"][i];
            assert_eq!(common.borders, border(&expected["borders"]));
            for (a, e) in endpoints
                .iter()
                .zip(expected["endpoints"].as_array().unwrap())
            {
                check(a, e);
            }
            let before = temporal.clone();
            let frame = temporal.process(32, &common.borders, &endpoints).unwrap();
            let mut replay = before;
            assert_eq!(
                replay.process(32, &common.borders, &endpoints).unwrap(),
                frame
            );
            assert_eq!(replay, temporal);
            for (a, e) in frame
                .coefficients
                .iter()
                .zip(expected["coefficients"].as_array().unwrap())
            {
                check(a, e);
            }
            if i == 0 {
                assert_eq!(common.borders[0], 0);
                assert_eq!(frame.coefficients[0], endpoints[0]);
                assert_eq!(frame.coefficients[31], endpoints[2]);
            }
            if i == 1 {
                assert_eq!(frame.coefficients[31], endpoints[1]);
            }
            if i == 2 {
                assert!(endpoints.is_empty());
                assert!(frame.coefficients.iter().all(|c| c == temporal.previous()));
            }
        }
    }
}
#[test]
fn malformed_inputs_are_transactional_and_opposite_finite_extrema_do_not_overflow() {
    let b = common::Bands::Twenty;
    let mut state = temporal::State::new(b);
    let matrices = vec![mixing::ComplexMatrix::default(); 20];
    let checkpoint = state.clone();
    for (slots, borders, ends) in [
        (0, vec![], vec![]),
        (31, vec![], vec![]),
        (32, vec![1], vec![]),
        (32, vec![1, 1], vec![matrices.clone(); 2]),
        (32, vec![2, 1], vec![matrices.clone(); 2]),
        (32, vec![1], vec![matrices[..19].to_vec()]),
        (32, vec![0, 1, 2, 3, 4], vec![matrices.clone(); 5]),
    ] {
        assert!(state.process(slots, &borders, &ends).is_err());
        assert_eq!(state, checkpoint);
    }
    let mut bad = matrices.clone();
    bad[19].h22.im = f64::NAN;
    assert!(state.process(32, &[31], &[bad.clone()]).is_err());
    assert_eq!(state, checkpoint);
    assert!(state.replace_previous(b, &bad).is_err());
    assert_eq!(state, checkpoint);
    for sign in [-1.0, 1.0] {
        for target in [-1.0, 1.0] {
            let previous = vec![
                mixing::ComplexMatrix {
                    h11: Complex {
                        re: f64::MAX * sign,
                        im: 0.0
                    },
                    ..mixing::ComplexMatrix::default()
                };
                20
            ];
            let endpoint = vec![
                mixing::ComplexMatrix {
                    h11: Complex {
                        re: f64::MAX * target,
                        im: 0.0
                    },
                    ..mixing::ComplexMatrix::default()
                };
                20
            ];
            state.replace_previous(b, &previous).unwrap();
            let frame = state.process(32, &[31], &[endpoint]).unwrap();
            assert!(
                frame
                    .coefficients
                    .iter()
                    .flatten()
                    .all(|m| m.h11.re.is_finite())
            );
        }
    }
}

#[test]
fn startup_and_frame_boundary_hold_exact_anchors_with_explicit_reconfiguration() {
    for slots in [24, 30, 32] {
        let b = common::Bands::Twenty;
        let mut state = temporal::State::new(b);
        let target = vec![
            mixing::ComplexMatrix {
                h11: Complex { re: 1.0, im: 0.5 },
                h12: Complex {
                    re: 0.75,
                    im: -0.25
                },
                ..mixing::ComplexMatrix::default()
            };
            20
        ];
        let frame = state
            .process(slots, &[slots - 1], &[target.clone()])
            .unwrap();
        assert!(
            frame.coefficients[0]
                .iter()
                .all(|m| *m == mixing::ComplexMatrix::default())
        );
        assert_eq!(frame.coefficients[usize::from(slots) - 1], target);
        let next = vec![mixing::ComplexMatrix::default(); 20];
        let frame = state.process(slots, &[slots - 1], &[next.clone()]).unwrap();
        assert_eq!(frame.coefficients[0], target);
        assert_eq!(frame.coefficients[usize::from(slots) - 1], next);
        let before = state.clone();
        assert!(
            state
                .replace_previous(common::Bands::ThirtyFour, &target)
                .is_err()
        );
        assert_eq!(state, before);
        // Geometry transfer is explicit: the caller supplies its already
        // mapped/reset complex boundary, never old native indices.
        let mapped = vec![target[0]; 34];
        state
            .replace_previous(common::Bands::ThirtyFour, &mapped)
            .unwrap();
        assert_eq!(state.bands(), common::Bands::ThirtyFour);
        let frame = state.process(slots, &[], &[]).unwrap();
        assert!(frame.coefficients.iter().all(|v| *v == mapped));
    }
}
