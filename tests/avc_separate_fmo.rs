//! Refusal tests, not playback acceptance: High 4:4:4 Predictive forbids FMO.
use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_plane_fmo_reports_a_profile_violation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-fmo");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 10);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        // The forbidden FMO combination is the intended violation; the NALs
        // must not introduce an additional arbitrary-slice-order violation.
        use fvid::codec::{
            avc::{Pps, Sps},
            avc_slice::SliceHeader,
            config::{AvcConfig, NalUnits},
        };
        let parameters = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(parameters.sps[0]).unwrap();
        let pps = Pps::parse(parameters.pps[0], &sps).unwrap();
        let mut previous = [None; 3];
        for nal in NalUnits::new(&packet, 4).unwrap() {
            let header = SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
            let prior = &mut previous[header.colour_plane_id as usize];
            assert!(
                prior.is_none_or(|address| address < header.first_mb),
                "{name}: unintended ASO"
            );
            *prior = Some(header.first_mb);
        }
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        let error = decoder.decode_order(&packet).unwrap_err();
        assert!(matches!(error, fvid::Error::Invalid(_)), "{name}: {error}");
        assert!(
            error.to_string().contains("forbids FMO slice groups"),
            "{name}: {error}"
        );
    }
}
