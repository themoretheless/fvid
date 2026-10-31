use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_native::{Core, Decoder, Stamp},
};
use serde_json::Value;
const ERROR: &str = "residual coding is not permitted in LD-MPS";
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-profile")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn packet<'a>(blob: &'a [u8], v: &Value) -> &'a [u8] {
    &blob[n(&v["offset"])..n(&v["offset"]) + n(&v["bytes"])]
}
#[test]
fn initial_and_late_nonconforming_profiles_refuse_without_publishing_bad_pcm() {
    let mut control = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join("control.mp4")).unwrap(),
        &mut control,
    )
    .unwrap();
    assert!(control.iter().any(|&v| v != 0));
    let m = manifest();
    Config::parse(&hex(&m["reserved_ssc"]))
        .unwrap()
        .validate_low_delay_profile()
        .unwrap();
    let mut reserved_control = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join("reserved-extension-control.mp4")).unwrap(),
        &mut reserved_control,
    )
    .unwrap();
    assert_eq!(reserved_control, control);
    let mut cases = vec![("initial-residual.mp4", true), ("late-residual.mp4", false)];
    for variant in m["variants"].as_array().unwrap() {
        let cfg = Config::parse(&hex(&variant["ssc"])).unwrap();
        assert_eq!(cfg.extensions[0].kind, n(&variant["kind"]) as u8);
        // A complete syntactic frame isolates profile validation from truncation.
        let payload = hex(&variant["spatial_frame"]);
        let mut bits = fvid_media::owned_aac::bits::BitReader::new(&payload);
        fvid_media::owned_aac::aac_ld_mps_frame::Decoder::new(&cfg)
            .unwrap()
            .decode(&mut bits)
            .unwrap();
        assert_eq!(bits.remaining(), 0, "{}", variant["name"]);
        assert!(
            cfg.validate_low_delay_profile()
                .unwrap_err()
                .to_string()
                .contains(ERROR)
        );
        cases.push((variant["initial"].as_str().unwrap(), true));
        cases.push((variant["late"].as_str().unwrap(), false));
    }
    for (file, initial) in cases {
        let data = std::fs::read(root().join(file)).unwrap();
        let mut pcm = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm).unwrap_err();
        assert!(e.to_string().contains(ERROR), "{file}: {e}");
        if initial {
            assert!(pcm.is_empty());
        } else {
            assert!(pcm.len() <= 2 * 512 * 2 * 4);
            assert_eq!(pcm, control[..pcm.len()]);
        }
    }
}
#[test]
fn late_profile_validation_preserves_native_history_and_allows_valid_retry() {
    let m = manifest();
    let cfg = Config::parse(&hex(&m["ordinary_ssc"])).unwrap();
    cfg.validate_low_delay_profile().unwrap();
    let core = Core {
        object_type: 39,
        sample_rate: 48000,
        frame_samples: 512,
        channels: 1,
        has_sbr: false,
        downscale: 1,
    };
    let mut d = Decoder::new(&cfg, core).unwrap();
    let mut control = d.clone();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let gold = std::fs::read(root().join("core.f32le")).unwrap();
    for (i, row) in m["good"].as_array().unwrap().iter().enumerate() {
        let samples = gold[i * 512 * 4..(i + 1) * 512 * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        let stamp = Stamp {
            pts: (i * 512) as i64,
            duration: 512,
            timescale: 48000,
        };
        let good = packet(&blob, row);
        let used = n(&row["core_bits"]);
        let want = control.decode_core(good, used, &samples, stamp).unwrap();
        if i == n(&m["bad_index"]) {
            let before = d.clone();
            let bad = &m["late"][i];
            let e = d
                .decode_core(packet(&blob, bad), n(&bad["core_bits"]), &samples, stamp)
                .unwrap_err();
            assert!(e.to_string().contains(ERROR));
            assert_eq!(d, before);
            for variant in m["variants"].as_array().unwrap() {
                let bad = &variant["row"];
                let e = d
                    .decode_core(packet(&blob, bad), n(&bad["core_bits"]), &samples, stamp)
                    .unwrap_err();
                assert!(e.to_string().contains(ERROR), "{}: {e}", variant["name"]);
                assert_eq!(d, before);
            }
        }
        assert_eq!(d.decode_core(good, used, &samples, stamp).unwrap(), want);
    }
    assert_eq!(d.finish().unwrap(), control.finish().unwrap());
}
