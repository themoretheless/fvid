use fvid::codec::{avc_decoder::AvcDecoder, avc_transform::switching_luma_4x4};
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-switching-luma.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn sp_si_luma_matches_independent_matrix_oracle_at_all_switching_qps() {
    for c in manifest()["cases"].as_array().unwrap() {
        let p = std::array::from_fn(|i| c["prediction"][i].as_u64().unwrap() as u16);
        let r = std::array::from_fn(|i| c["levels"][i].as_i64().unwrap() as i32);
        let expected: [u16; 16] =
            std::array::from_fn(|i| c["expected"][i].as_u64().unwrap() as u16);
        assert_eq!(
            switching_luma_4x4(
                &p,
                &r,
                c["qp"].as_u64().unwrap() as u8,
                c["qs"].as_u64().unwrap() as u8,
                c["switching"].as_bool().unwrap()
            )
            .unwrap(),
            expected,
            "{c}"
        );
    }
}
#[test]
fn switching_luma_rejects_invalid_dimensions_and_levels() {
    assert!(switching_luma_4x4(&[256; 16], &[0; 16], 0, 0, true).is_err());
    assert!(switching_luma_4x4(&[128; 16], &[0; 16], 52, 0, true).is_err());
    assert!(switching_luma_4x4(&[128; 16], &[0; 16], 0, 52, false).is_err());
    assert!(switching_luma_4x4(&[128; 16], &[i32::MAX; 16], 0, 0, true).is_err());
}
#[test]
fn si_video_specific_refusal_is_not_playback_acceptance() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_slice::{SliceHeader, SliceType},
        config::AvcConfig,
    };
    let m = manifest();
    let config = hex(m["video"]["configuration"].as_str().unwrap());
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut control = AvcDecoder::new(&config, 1 << 20).unwrap();
    for (index, p) in m["control"]["packets"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let frame = control.decode(&hex(p.as_str().unwrap())).unwrap().unwrap();
        assert!(
            frame
                .y
                .iter()
                .enumerate()
                .all(|(i, v)| *v == ((i * 13 + index * 37) % 256) as u16)
        );
    }
    let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
    let packet = hex(m["video"]["packets"][0].as_str().unwrap());
    let h = SliceHeader::parse(&packet[4..], &sps, &pps).unwrap();
    assert_eq!(h.slice_type, SliceType::Si);
    assert_eq!(h.slice_qs, Some(0));
    let error = decoder.decode(&packet).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("AVC picture type is not implemented"),
        "{error}"
    );
}
