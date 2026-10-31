const CORPUS: &str = "aac-eld-mps128-ges";
const PROGRAMMES: usize = 396;
include!("support/aac_eld_mps128_suite.rs");

#[test]
fn active_ges_changes_pcm_for_every_high_rate_tree_and_core_geometry() {
    let m = manifest();
    let ordinary = root().parent().unwrap().join("aac-eld-mps128");
    let mut changed = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "plain")
    {
        let file = c["video"].as_str().unwrap();
        let mut shaped = Vec::new();
        let mut control = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(file)).unwrap(),
            &mut shaped,
        )
        .unwrap();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(ordinary.join(file)).unwrap(),
            &mut control,
        )
        .unwrap();
        assert_eq!(shaped.len(), control.len());
        let maximum = shaped
            .chunks_exact(4)
            .zip(control.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(
            maximum > 1e-10,
            "{} GES had no signal effect: {maximum}",
            c["name"]
        );
        changed += 1;
    }
    assert_eq!(changed, 66);
}
