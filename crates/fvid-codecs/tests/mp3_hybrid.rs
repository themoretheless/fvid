use fvid_codecs::mp3_hybrid::{Hybrid, Window};
fn window(code: u8) -> Window {
    match code {
        0 => Window::Normal,
        1 => Window::Start,
        2 => Window::Short,
        3 => Window::Stop,
        _ => panic!("invalid fixture window"),
    }
}
#[test]
fn hybrid_basis_mixed_windows_and_transitions_match_absolute_timeline_oracle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-hybrid");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "case") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let granules = u16::from_be_bytes(bytes[..2].try_into().unwrap()) as usize;
        let expected = std::fs::read(path.with_extension("f64le")).unwrap();
        let expected: Vec<f64> = expected
            .chunks_exact(8)
            .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(bytes.len(), 2 + granules * (2 + 576 * 8));
        assert_eq!(expected.len(), granules * 576);
        let mut hybrid = Hybrid::default();
        for _ in 0..2 {
            hybrid.reset();
            let mut at = 2;
            for g in 0..granules {
                let kind = window(bytes[at]);
                let mixed = bytes[at + 1];
                at += 2;
                let input = std::array::from_fn(|i| {
                    f64::from_le_bytes(bytes[at + i * 8..at + i * 8 + 8].try_into().unwrap())
                });
                at += 576 * 8;
                let actual = hybrid
                    .process(&input, kind, mixed)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                for (i, (a, b)) in actual
                    .iter()
                    .zip(&expected[g * 576..g * 576 + 576])
                    .enumerate()
                {
                    assert!(
                        (a - b).abs() < 3e-11,
                        "{} granule {g} sample {i}: {a} != {b}",
                        path.display()
                    );
                }
            }
        }
        count += 1;
    }
    assert_eq!(count, 803);
}
#[test]
fn failed_hybrid_granule_preserves_both_overlap_and_polyphase_history() {
    let mut hybrid = Hybrid::default();
    let mut control = hybrid.clone();
    let input = std::array::from_fn(|i| ((i * 7 % 31) as f64 - 15.) / 4096.);
    assert_eq!(
        hybrid.process(&input, Window::Start, 2).unwrap(),
        control.process(&input, Window::Start, 2).unwrap()
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
        assert!(hybrid.process(&[value; 576], Window::Short, 0).is_err());
    }
    assert!(hybrid.process(&input, Window::Short, 1).is_err());
    assert_eq!(
        hybrid.process(&input, Window::Short, 2).unwrap(),
        control.process(&input, Window::Short, 2).unwrap()
    );
    hybrid.reset();
    control.reset();
    assert_eq!(
        hybrid.process(&input, Window::Normal, 0).unwrap(),
        control.process(&input, Window::Normal, 0).unwrap()
    );
}
