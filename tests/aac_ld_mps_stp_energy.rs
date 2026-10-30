//! Frozen independent scalar STP oracle. No reference compiler, media converter
//! or network access is involved in test execution. This is not PCM acceptance.
use fvid_media::owned_aac::aac_ld_mps_stp_energy::EnergyTracker;
use serde_json::Value;

#[test]
fn iso_scalar_energy_gains_history_refresh_and_checkpoint_replay() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/aac-ld-mps-stp-energy/reference.json"
    ))
    .unwrap();
    let mut compared = 0;
    let mut maximum_error = 0f64;
    let mut modified = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let tree = case["tree"].as_u64().unwrap();
        let groups: Vec<Option<usize>> = match tree {
            0 => vec![Some(0), Some(0), Some(0), None, Some(0), Some(0)],
            1 => vec![Some(0), Some(0), Some(0), Some(0), Some(0), None],
            2 => vec![Some(0), Some(0), Some(1), Some(1), None, None],
            3 | 4 => vec![
                Some(0),
                Some(0),
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                None,
                None,
            ],
            _ => panic!("unqualified tree"),
        };
        let mut decoder = EnergyTracker::new(if tree < 2 { 1 } else { 2 }, &groups).unwrap();
        let initial = decoder.clone();
        let mut saved = None;
        let mut output = Vec::new();
        for (slot, input) in case["slots"].as_array().unwrap().iter().enumerate() {
            let vector = |key: &str| {
                input[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect::<Vec<_>>()
            };
            let dry = vector("dry");
            let wet = vector("wet");
            if slot == 32 {
                saved = Some(decoder.clone());
            }
            let gains = decoder.step(&dry, &wet).unwrap();
            let expected = vector("gain");
            for (row, (&actual, reference)) in gains.iter().zip(expected).enumerate() {
                let error = (actual - reference).abs();
                maximum_error = maximum_error.max(error);
                assert!(
                    error < 2e-6,
                    "tree {tree}, pattern {}, slot {slot}, row {row}: {actual} vs {reference}, error {error}",
                    case["pattern"]
                );
                if groups[row].is_none() {
                    assert_eq!(actual, 1.);
                }
                if (actual - 1.).abs() > 0.01 {
                    modified += 1;
                }
                compared += 1;
            }
            output.push(gains);
        }
        let mut checkpoint = saved.unwrap();
        for (slot, input) in case["slots"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .skip(32)
        {
            let vector = |key: &str| {
                input[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                checkpoint.step(&vector("dry"), &vector("wet")).unwrap(),
                output[slot]
            );
        }
        assert_eq!(checkpoint, decoder);
        decoder.reset();
        assert_eq!(decoder, initial);
        let bytes = decoder.retained_payload_bytes().unwrap();
        assert!(bytes > 0 && bytes < 1024);
    }
    assert_eq!(compared, 13_600);
    assert!(modified > 5000, "oracle must exercise non-identity gains");
    eprintln!("{compared} scalar STP gains, maximum error {maximum_error}, modified {modified}");
}
