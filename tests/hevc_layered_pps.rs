//! Metadata acceptance and scalar colour mapping; not multilayer picture playback.
use fvid::codec::{
    hevc_pps::{LayeredPps, LayeredPpsContext},
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
#[test]
fn layered_pps_metadata_and_colour_mapping_regressions() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-layered-pps.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let prefix = c["prefix"].as_str().unwrap();
        let read = |suffix: &str| data(&format!("{prefix}.{suffix}"));
        let vps = LayeredVps::parse(&read("vps"), 1 << 20).unwrap();
        let base = LayeredSps::parse(
            &read("base.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 0,
                output_layer_set: 0,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap();
        let current = LayeredSps::parse(
            &read("current.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 3,
                output_layer_set: 1,
                active_sps: std::slice::from_ref(&base),
            },
            1 << 20,
        )
        .unwrap();
        let references = [base, current];
        let base_pps = LayeredPps::parse(
            &read("base.pps"),
            &LayeredPpsContext {
                vps: &vps,
                sps: &references[0],
                active_sps: &references,
                active_pps: &[],
            },
            1 << 20,
        )
        .unwrap_or_else(|e| panic!("{prefix} base PPS: {e}"));
        let active_pps = [base_pps];
        let selected = usize::from(!c["base_context"].as_bool().unwrap());
        let context = LayeredPpsContext {
            vps: &vps,
            sps: &references[selected],
            active_sps: &references,
            active_pps: &active_pps,
        };
        let raw = read("pps");
        let movie = Mp4Reader::open(std::io::Cursor::new(read("mp4")), Limits::default()).unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let configured = config
            .arrays
            .iter()
            .find(|a| a.nal_type == 34)
            .unwrap()
            .units
            .last()
            .unwrap();
        assert_eq!(*configured, raw);
        let result = LayeredPps::parse(&raw, &context, 1 << 20);
        if let Some(message) = c["error"].as_str() {
            let error = result.unwrap_err();
            assert!(
                if c["unsupported"].as_bool().unwrap() {
                    matches!(error, fvid::Error::Unsupported(_))
                } else {
                    matches!(error, fvid::Error::Invalid(_))
                },
                "{prefix}: {error}"
            );
            assert!(
                error.to_string().contains(message),
                "{prefix}: {error}, expected {message}"
            );
            continue;
        }
        let p = result.unwrap_or_else(|e| panic!("{prefix}: {e}"));
        assert_eq!(LayeredPps::parse(configured, &context, 1 << 20).unwrap(), p);
        assert_eq!(p.layer_id, references[selected].layer_id);
        assert!(p.pps.poc_reset_info_present);
        assert_eq!(p.scaling_list_data_present, c["scaling"] == "explicit");
        assert_eq!(
            p.scaling_list_reference,
            if c["scaling"] == "infer" {
                Some(0)
            } else {
                None
            }
        );
        assert_eq!(
            u64::from(p.effective_scaling_lists.matrix(0, 0).unwrap().coefficients[0]),
            c["first_coefficient"].as_u64().unwrap(),
            "{prefix}"
        );
        assert_eq!(
            p.reference_locations.len(),
            c["locations"].as_array().unwrap().len()
        );
        for (loc, expected) in p
            .reference_locations
            .iter()
            .zip(c["locations"].as_array().unwrap())
        {
            assert_eq!(u64::from(loc.layer_id), expected["id"].as_u64().unwrap());
            for (key, actual) in [
                ("scaled", loc.scaled_offsets),
                ("region", loc.region_offsets),
            ] {
                assert_eq!(
                    actual.map(|v| v.to_vec()),
                    expected[key]
                        .as_array()
                        .map(|v| v.iter().map(|x| x.as_i64().unwrap() as i32).collect()),
                    "{prefix}"
                );
            }
            let reference = references
                .iter()
                .find(|s| s.layer_id == loc.layer_id)
                .unwrap();
            let geometry = loc
                .geometry(&references[selected].sps, &reference.sps, selected == 1)
                .unwrap();
            if prefix.ends_with("default-phase") {
                assert_eq!(geometry.phases, [0, 0, 0, 2]);
            }
            if prefix.ends_with("444-phase") {
                assert_eq!(geometry.phases, [0, 0, 0, 0]);
            }
            if prefix.ends_with("phase-extremes") {
                assert_eq!(geometry.phases, [31, 0, 55, -8]);
            }
            if prefix.ends_with("ignored-phase") {
                assert_eq!(geometry.phases, [0, 0, 0, -1]);
            }
            if prefix.ends_with("locations") {
                assert_eq!(geometry.scaled_size, [248, 188]);
                assert_eq!(geometry.reference_size, [124, 132]);
                assert_eq!(geometry.phases, [0, 0, 0, 2]);
            }
        }
        if !c["oracle"].is_null() {
            let oracle = &c["oracle"];
            let table = p.colour_mapping.as_ref().unwrap();
            assert!(table.map_sample(3, [0; 3]).is_err());
            if table.input_depth[0] < 16 {
                assert!(
                    table
                        .map_sample(0, [1 << table.input_depth[0], 0, 0])
                        .is_err()
                );
            }
            if prefix.ends_with("cm-part-1-2-1") {
                let error = LayeredPps::parse(&raw, &context, raw.len() + 3500).unwrap_err();
                assert!(error.to_string().contains("budget"), "{error}");
            }
            let dims = table.dimensions();
            assert_eq!(serde_json::to_value(dims).unwrap(), oracle["dimensions"]);
            for y in 0..dims[0] {
                for cb in 0..dims[1] {
                    for cr in 0..dims[2] {
                        assert_eq!(
                            serde_json::to_value(table.coefficients([y, cb, cr]).unwrap()).unwrap(),
                            oracle["coefficients"][(y * dims[1] + cb) * dims[2] + cr],
                            "{prefix} cell {y}/{cb}/{cr}"
                        );
                    }
                }
            }
            for probe in oracle["probes"].as_array().unwrap() {
                let input = std::array::from_fn(|i| probe[0][i].as_u64().unwrap() as u16);
                for component in 0..3 {
                    assert_eq!(
                        u64::from(table.map_sample(component, input).unwrap()),
                        probe[1][component].as_u64().unwrap(),
                        "{prefix} {input:?}/{component}"
                    );
                }
            }
        } else {
            assert!(p.colour_mapping.is_none());
        }
        if !c["future"].as_bool().unwrap() {
            for end in 0..raw.len() {
                assert!(
                    LayeredPps::parse(&raw[..end], &context, 1 << 20).is_err(),
                    "{prefix} truncation {end}"
                );
            }
        }
    }
}
