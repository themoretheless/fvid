use fvid_codecs::mpeg_synthesis::Synthesis;
fn read(path: &std::path::Path) -> Vec<f64> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(8)
        .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn all_subbands_history_and_reset_match_independent_convolution() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mpeg-synthesis");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "bands") {
            continue;
        }
        let inputs = read(&path);
        let expected = read(&path.with_extension("f64le"));
        assert_eq!(inputs.len(), expected.len());
        let mut filter = Synthesis::default();
        for _ in 0..2 {
            filter.reset();
            for (slot, input) in inputs.chunks_exact(32).enumerate() {
                let actual = filter.push(input.try_into().unwrap()).unwrap();
                for (j, v) in actual.iter().enumerate() {
                    let want = expected[slot * 32 + j];
                    assert!(
                        (v - want).abs() < 2e-12,
                        "{} slot {slot}, sample {j}: {v} != {want}",
                        path.display()
                    );
                }
            }
        }
        count += 1;
    }
    assert_eq!(count, 34);
}
#[test]
fn rejected_nonfinite_input_does_not_change_synthesis_history() {
    let mut filter = Synthesis::default();
    let mut control = filter.clone();
    let input = std::array::from_fn(|i| i as f64 / 128.);
    assert_eq!(filter.push(&input).unwrap(), control.push(&input).unwrap());
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
        let bad = [value; 32];
        assert!(filter.push(&bad).is_err());
    }
    assert_eq!(filter.push(&input).unwrap(), control.push(&input).unwrap());
}
