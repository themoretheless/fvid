const CORPUS: &str = "aac-ld-mps-hrtf-all-grids";
const CONTROLS: usize = 1;
const PROGRAMMES: usize = 7;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn all49_grid_pairs_parse_three_complete_spatial_frames_and_isolate_the_grid_refusal() {
    use fvid_media::owned_aac::{aac_ld_mps_frame, AacDecodeOptions, NativeAacDecoder};
    let manifest = m();
    let mut pairs = std::collections::BTreeSet::new();
    for c in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(manifest["errors"].as_array().unwrap())
    {
        assert!(pairs.insert((n(&c["spatial_bands"]), n(&c["hrtf_bands"]))));
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        cfg.validate_low_delay_profile().unwrap();
        assert_eq!(usize::from(cfg.parameter_bands), n(&c["spatial_bands"]));
        assert_eq!(
            usize::from(cfg.hrtf.as_ref().unwrap().bands),
            n(&c["hrtf_bands"])
        );
        let mut syntax = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
        assert_eq!(c["reference"].as_array().unwrap().len(), 3);
        for wire in c["reference"].as_array().unwrap() {
            let bytes = hex(wire);
            let mut b = BitReader::new(&bytes);
            let f = syntax.decode(&mut b).unwrap();
            assert_eq!(f.slots.len(), 2);
            assert!(b.remaining() < 8);
        }
        if c["acceptance"] == true {
            continue;
        }
        for mode in [Mode::Real, Mode::Complex] {
            let err = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .err()
            .expect("unequal grid is not playback acceptance")
            .to_string();
            assert!(
                err.contains("HRTF grid conversion is not yet qualified"),
                "{}: {err}",
                c["name"]
            );
        }
        let mut output = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut output)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("HRTF grid conversion is not yet qualified"),
            "{e}"
        );
        assert!(output.is_empty());
        let e = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(&c["file"])),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("HRTF grid conversion is not yet qualified"),
            "{e}"
        );
        assert!(output.is_empty());
    }
    assert_eq!(pairs.len(), 49);
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 42);
}
#[test]
fn every_equal_grid_keeps_both_decorrelation_modes_against_saved_core_composition() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder};
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let mut maximum = 0f32;
    let mut values = 0usize;
    for c in m()["cases"].as_array().unwrap() {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let core = read(&c["core_gold"]);
        let count = n(&c["out_n"]);
        for mode in [Mode::Real, Mode::Complex] {
            let mut stage = aac_ld_mps_pcm::Decoder::new(&cfg, mode).unwrap();
            let mut expected = Vec::new();
            for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
                let planar = (0..2)
                    .map(|ch| {
                        (0..count)
                            .map(|t| {
                                let at = ((i * count + t) * 2 + ch) * 4;
                                f64::from(f32::from_le_bytes(core[at..at + 4].try_into().unwrap()))
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let pcm = stage
                    .decode(&mut BitReader::new(&hex(wire)), &planar, &vec![None; 4])
                    .unwrap();
                expected.extend(pcm.samples.iter().flat_map(|v| (*v as f32).to_le_bytes()));
            }
            let mut decoder = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut actual = Vec::new();
            for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = n(&r["offset"]);
                if let Some(f) = decoder
                    .decode_timed(
                        &blob[at..at + n(&r["bytes"])],
                        (i * count) as i64,
                        count as u64,
                    )
                    .unwrap()
                {
                    actual.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
                }
            }
            while let Some(f) = decoder.finish().unwrap() {
                actual.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            maximum = maximum.max(near(&actual, &expected, 1e-7));
            values += actual.len() / 4;
        }
    }
    eprintln!("All-grid equal controls: {values} scalars, staged PCM max error {maximum}");
}
