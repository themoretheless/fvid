//! Parsed Annex F access-unit state acceptance; not scalable reconstruction.
use fvid::codec::{
    hevc_poc::{LayeredPocState, PocPicture},
    hevc_pps::{LayeredPps, LayeredPpsContext},
    hevc_slice::{LayeredSliceContext, SliceHeader},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
struct Parsed {
    vps: LayeredVps,
    sps: LayeredSps,
    pps: LayeredPps,
    header: SliceHeader,
}
fn load(c: &serde_json::Value) -> Parsed {
    let vps = LayeredVps::parse(&data(c["vps"].as_str().unwrap()), 1 << 20).unwrap();
    let sps = LayeredSps::parse(
        &data(c["sps"].as_str().unwrap()),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: c["layer"].as_u64().unwrap() as u8,
            output_layer_set: 1,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let pps = LayeredPps::parse(
        &data(c["pps"].as_str().unwrap()),
        &LayeredPpsContext {
            vps: &vps,
            sps: &sps,
            active_sps: &[],
            active_pps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let header = SliceHeader::parse_layered(
        &data(&format!("{}.nal", c["prefix"].as_str().unwrap())),
        &LayeredSliceContext {
            vps: &vps,
            sps: &sps,
            pps: &pps,
        },
        1 << 20,
    )
    .unwrap();
    Parsed {
        vps,
        sps,
        pps,
        header,
    }
}
#[test]
fn layered_access_units_plan_pocs_and_dpb_decrements_atomically() {
    let m: serde_json::Value = serde_json::from_slice(&data("hevc-layered-slices.json")).unwrap();
    let cases: std::collections::HashMap<_, _> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["prefix"].as_str().unwrap(), c))
        .collect();
    assert_eq!(m["state_sequences"].as_array().unwrap().len(), 24);
    for sequence in m["state_sequences"].as_array().unwrap() {
        let label = sequence["label"].as_str().unwrap();
        let mut state = LayeredPocState::default();
        for (index, unit) in sequence["units"].as_array().unwrap().iter().enumerate() {
            let records: Vec<_> = unit["headers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| load(cases[p.as_str().unwrap()]))
                .collect();
            let contexts: Vec<_> = records
                .iter()
                .map(|p| LayeredSliceContext {
                    vps: &p.vps,
                    sps: &p.sps,
                    pps: &p.pps,
                })
                .collect();
            let pictures: Vec<_> = records
                .iter()
                .zip(&contexts)
                .map(|(p, c)| PocPicture {
                    header: &p.header,
                    context: c,
                })
                .collect();
            let before = state.clone();
            let result = state.preview(&records[0].vps, &pictures);
            if let Some(error) = unit["error"].as_str() {
                let failure = result.unwrap_err();
                assert!(
                    matches!(failure, fvid::Error::Invalid(_)),
                    "{label}: {failure}"
                );
                assert!(
                    failure.to_string().contains(error),
                    "{label} AU{index}: {failure}; expected {error}"
                );
                assert_eq!(state, before, "{label}: failed preview mutated state");
                continue;
            }
            let plan = result.unwrap_or_else(|e| panic!("{label} AU{index}: {e}"));
            assert_eq!(
                state, before,
                "{label}: successful preview published prematurely"
            );
            let outputs: Vec<_> = plan.pictures().collect();
            assert_eq!(outputs.len(), pictures.len());
            for p in &outputs {
                assert_eq!(
                    p.resetting,
                    unit["resetting"][p.layer_id.to_string()].as_bool().unwrap(),
                    "{label} AU{index} layer{} resetting",
                    p.layer_id
                );
                assert_eq!(
                    i64::from(p.poc),
                    unit["poc"].as_i64().unwrap(),
                    "{label} AU{index} layer{}",
                    p.layer_id
                );
            }
            for layer in 0..63 {
                assert_eq!(
                    plan.decrement(layer).unwrap(),
                    unit["decrements"][layer.to_string()].as_i64().unwrap_or(0),
                    "{label} AU{index} layer{layer} delta"
                );
            }
            assert!(plan.decrement(63).is_err());
            state = plan.into_state();
            for (layer, expected) in unit["previous"].as_object().unwrap() {
                assert_eq!(
                    state.previous(layer.parse().unwrap()),
                    Some(expected.as_i64().unwrap() as i32),
                    "{label} AU{index} previous layer{layer}"
                );
            }
            if index == 0 {
                let reverse: Vec<_> = pictures
                    .iter()
                    .rev()
                    .map(|p| PocPicture {
                        header: p.header,
                        context: p.context,
                    })
                    .collect();
                if reverse.len() > 1 {
                    assert!(
                        LayeredPocState::default()
                            .preview(&records[0].vps, &reverse)
                            .unwrap_err()
                            .to_string()
                            .contains("increasing layer order")
                    );
                }
                let duplicate = [
                    PocPicture {
                        header: pictures[0].header,
                        context: pictures[0].context,
                    },
                    PocPicture {
                        header: pictures[0].header,
                        context: pictures[0].context,
                    },
                ];
                assert!(
                    LayeredPocState::default()
                        .preview(&records[0].vps, &duplicate)
                        .is_err()
                );
                let mut changed = records[0].vps.clone();
                changed
                    .extension
                    .as_mut()
                    .unwrap()
                    .representations
                    .poc_lsb_aligned ^= true;
                assert!(
                    state
                        .preview(&changed, &pictures)
                        .unwrap_err()
                        .to_string()
                        .contains("configuration requires reset")
                );
            }
        }
    }
}
