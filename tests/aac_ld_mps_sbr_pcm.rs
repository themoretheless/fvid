const CORPUS: &str = "aac-ld-mps-sbr-pcm";
const CONTROLS: usize = 55;
const PROGRAMMES: usize = 152;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn sbr_crc_failure_before_spatial_input_keeps_both_decoder_histories() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let errors = manifest["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    for bad in errors {
        let c = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == bad["control"])
            .unwrap();
        let asc = hex(&c["asc"]);
        let rate = n(&c["rate"]) as u32;
        let mut prefix = Vec::new();
        let e =
            fvid::native_media::decode_mp4_aac_pcm(&read(&bad["video"]), &mut prefix).unwrap_err();
        assert!(e.to_string().contains("low-delay SBR CRC mismatch"), "{e}");
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut full).unwrap();
        assert!(prefix.len() < full.len());
        assert_eq!(prefix, full[..prefix.len()]);
        let mut d = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        let mut control = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&row["offset"]);
            let packet = &blob[at..at + n(&row["bytes"])];
            let pts = (i * n(&c["out_n"])) as i64;
            let duration = n(&c["out_n"]) as u64;
            if i == n(&bad["bad_index"]) {
                let b = &bad["frames"][i];
                let at = n(&b["offset"]);
                let e = d
                    .decode_timed_in_clock(&blob[at..at + n(&b["bytes"])], pts, duration, rate)
                    .unwrap_err();
                assert!(e.to_string().contains("low-delay SBR CRC mismatch"));
            }
            assert_eq!(
                d.decode_timed_in_clock(packet, pts, duration, rate)
                    .unwrap(),
                control
                    .decode_timed_in_clock(packet, pts, duration, rate)
                    .unwrap()
            );
        }
        loop {
            let a = d.finish().unwrap();
            let b = control.finish().unwrap();
            assert_eq!(a, b);
            if a.is_none() {
                break;
            }
        }
    }
}
