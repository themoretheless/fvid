//! Complete hybrid QMF acceptance; full native PS PCM remains a separate gate.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_history,
    aac_ps_hybrid::{State, synthesize},
    aac_ps_mapping::Bands,
    aac_ps_matrix_controller, aac_sbr_history,
    aac_sbr_qmf::Complex,
    bits::BitReader,
};
use serde_json::Value;
const DATA: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-hybrid-reference.bin");
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-hybrid-oracles.json"
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
fn scalars(v: &Value) -> Vec<f64> {
    let offset = v[0].as_u64().unwrap() as usize;
    let count = v[1].as_u64().unwrap() as usize;
    DATA[offset..offset + count * 8]
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}
fn input(case: &Value) -> Vec<[Complex; 64]> {
    scalars(&case["input"])
        .chunks_exact(128)
        .map(|slot| {
            std::array::from_fn(|k| Complex {
                re: slot[k * 2],
                im: slot[k * 2 + 1],
            })
        })
        .collect()
}
fn compare(actual: impl IntoIterator<Item = f64>, descriptor: &Value) {
    let actual: Vec<_> = actual.into_iter().collect();
    let expected = scalars(descriptor);
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(a.is_finite());
        assert!((a - e).abs() < 2e-12, "{a} != {e}");
    }
}
fn check(frame: &fvid_media::owned_aac::aac_ps_hybrid::Frame, row: &Value) {
    assert_eq!(frame.bands, bands(&row["bands"]));
    assert_eq!(frame.slots.len(), row["slots"].as_u64().unwrap() as usize);
    for slot in &frame.slots {
        assert_eq!(slot.len(), frame.bands.bindings().len());
    }
    compare(
        frame.slots.iter().flatten().flat_map(|c| [c.re, c.im]),
        &row["output"],
    );
    compare(
        frame
            .synthesize()
            .unwrap()
            .iter()
            .flatten()
            .flat_map(|c| [c.re, c.im]),
        &row["synthesis"],
    );
}
#[test]
fn all_routed_bands_and_aligned_upper_channels_match_decimal_full_convolution() {
    for case in reference()["cases"].as_array().unwrap() {
        let input = input(case);
        let mut state = State::default();
        for row in case["frames"].as_array().unwrap() {
            let start = row["start"].as_u64().unwrap() as usize;
            let size = row["slots"].as_u64().unwrap() as usize;
            let old = state.bands();
            let frame = state
                .process(bands(&row["bands"]), &input[start..start + size])
                .unwrap();
            assert_eq!(frame.bands_changed, old != frame.bands);
            check(&frame, row);
            for (n, slot) in frame.synthesize().unwrap().iter().enumerate() {
                for (k, &actual) in slot.iter().enumerate() {
                    let expected = if start + n < 6 {
                        Complex::default()
                    } else {
                        input[start + n - 6][k]
                    };
                    assert!((actual.re - expected.re).abs() < 2e-12);
                    assert!((actual.im - expected.im).abs() < 2e-12);
                }
            }
        }
    }
}
#[test]
fn chunking_replay_reset_and_empty_calls_preserve_shared_history_across_grids() {
    for case in reference()["cases"].as_array().unwrap() {
        let input = input(case);
        let mut whole = State::default();
        let rows = case["frames"].as_array().unwrap();
        let mut complete = vec![];
        for row in rows {
            let start = row["start"].as_u64().unwrap() as usize;
            let size = row["slots"].as_u64().unwrap() as usize;
            complete.push(
                whole
                    .process(bands(&row["bands"]), &input[start..start + size])
                    .unwrap()
                    .slots,
            );
        }
        for width in [1, 2, 7, 13, 24, 30, 32] {
            let mut state = State::default();
            for (row, expected) in rows.iter().zip(&complete) {
                let grid = bands(&row["bands"]);
                let start = row["start"].as_u64().unwrap() as usize;
                let size = row["slots"].as_u64().unwrap() as usize;
                let mut actual = vec![];
                for chunk in input[start..start + size].chunks(width) {
                    let before = state.clone();
                    let empty = state.process(grid, &[]).unwrap();
                    assert!(empty.slots.is_empty());
                    assert!(!empty.bands_changed);
                    assert_eq!(empty.bands, before.bands());
                    assert_eq!(before, state);
                    let mut replay = before;
                    let output = state.process(grid, chunk).unwrap();
                    assert_eq!(replay.process(grid, chunk).unwrap(), output);
                    assert_eq!(replay, state);
                    actual.extend(output.slots);
                }
                assert_eq!(&actual, expected);
            }
            assert_eq!(state, whole);
            for grid in [Bands::Twenty, Bands::ThirtyFour] {
                state.reset(grid);
                assert_eq!(state, State::new(grid));
            }
        }
    }
}
#[test]
fn failed_grid_changes_and_late_overflow_are_transactional() {
    let mut state = State::new(Bands::ThirtyFour);
    let before = state.clone();
    let mut samples = vec![[Complex::default(); 64]; 40];
    for (n, row) in samples.iter_mut().enumerate() {
        row[1].re = if n % 2 == 0 { f64::MAX } else { -f64::MAX };
    }
    assert!(state.process(Bands::Twenty, &samples).is_err());
    assert_eq!(state, before);
    for grid in [Bands::Twenty, Bands::ThirtyFour] {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut samples = vec![[Complex::default(); 64]; 32];
            samples[31][63].im = bad;
            assert!(state.process(grid, &samples).is_err());
            assert_eq!(state, before);
            let mut hybrid = vec![vec![Complex::default(); grid.bindings().len()]; 32];
            hybrid[31][0].re = bad;
            assert!(synthesize(grid, &hybrid).is_err());
        }
        assert!(synthesize(grid, &[vec![Complex::default(); grid.bindings().len() - 1]]).is_err());
        let mut hybrid = vec![Complex::default(); grid.bindings().len()];
        for (sample, binding) in hybrid.iter_mut().zip(grid.bindings()) {
            if binding.qmf == 0 {
                sample.re = f64::MAX;
            }
        }
        assert!(synthesize(grid, &[hybrid]).is_err());
    }
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn synthetic_mp4_grid_changes_drive_native_parameters_and_the_complete_hybrid_bank() {
    let refs = reference();
    let case = refs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "grid_video")
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
    let mut bank = State::default();
    let samples = input(case);
    for (n, row) in case["frames"].as_array().unwrap().iter().enumerate() {
        let packet = &v["packet_frames"][n];
        let off = packet["offset"].as_u64().unwrap() as usize;
        let size = packet["bytes"].as_u64().unwrap() as usize;
        let mut data = vec![];
        mp4.read_packet(ai, n, &mut data).unwrap();
        assert_eq!(data, &packets[off..off + size]);
        let raw = hex(v["sbr_payloads"][n].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        let f = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let p = ps
            .read_sbr_extensions(f.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        assert_eq!(p.len(), 1);
        let matrix = controller.process(&p[0].parameters, 32).unwrap();
        assert_eq!(matrix.temporal.bands, bands(&row["bands"]));
        let start = row["start"].as_u64().unwrap() as usize;
        let frame = bank
            .process(matrix.temporal.bands, &samples[start..start + 32])
            .unwrap();
        check(&frame, row);
        // Feed real analyzed mono subbands to the matrix path. Decorrelation is
        // deliberately absent here; zero diffuse input is not a PCM oracle.
        let diffuse = vec![vec![Complex::default(); frame.bands.bindings().len()]; 32];
        let stereo = matrix.mix(&frame.slots, &diffuse).unwrap();
        for channel in &stereo.channels {
            assert_eq!(synthesize(frame.bands, channel).unwrap().len(), 32);
        }
    }
}
