const CORPUS: &str = "aac-eld-residual64-geometries";
const CORE_FRAMES: usize = 48;
const PROGRAMMES: usize = 462;
include!("support/aac_eld_residual64_multicore_suite.rs");

#[test]
fn coded_residual_rates_and_extra_short_spectra_cover_each_integral_geometry() {
    use fvid_media::owned_aac::{aac_ld_mps_config::ExtensionValue, aac_synthesis::WindowSequence};
    let mut geometries = std::collections::BTreeSet::new();
    let mut grids = std::collections::BTreeSet::new();
    let mut extra = 0;
    let mut spectra = 0;
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "plain")
    {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let ext = &cfg.extensions[0].value;
        let ExtensionValue::Residual {
            rate, frames_code, ..
        } = ext
        else {
            panic!("missing residual extension")
        };
        assert_eq!(*rate as usize, n(&c["residual_rate"]));
        assert_eq!(*frames_code, 0);
        assert_eq!(n(&c["slots"]) * 64, n(&c["n"]) * n(&c["group"]));
        geometries.insert((n(&c["n"]), n(&c["group"]), n(&c["slots"]), *rate));
        grids.insert(cfg.parameter_bands);
        for row in c["reference"].as_array().unwrap() {
            let wire = hex(&row["residual"]);
            let parsed =
                aac_ld_mps_residual::read(&mut BitReader::new(&wire), &cfg, ext, 2).unwrap();
            for data in parsed.boxes.iter().flatten() {
                assert_eq!(data.blocks.len(), 1);
                for b in &data.blocks {
                    let expected = b.first.sequence == WindowSequence::EightShort
                        && matches!(n(&c["slots"]), 24 | 30);
                    assert_eq!(b.extra.is_some(), expected);
                    spectra += 1 + b.extra.is_some() as usize;
                    extra += b.extra.is_some() as usize;
                }
            }
        }
    }
    assert_eq!(
        geometries,
        [
            (480, 2, 15, 48000),
            (480, 4, 30, 24000),
            (512, 2, 16, 48000),
            (512, 3, 24, 32000),
            (512, 4, 32, 24000)
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(grids, [4, 5, 7, 9, 12, 15, 23].into_iter().collect());
    assert!(extra > 100);
    eprintln!("extended residual64 parsed spectra {spectra};extra short blocks {extra}");
}
