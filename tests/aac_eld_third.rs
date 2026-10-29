const CORPUS: &str = "aac-eld-third";
const CASES: usize = 44;
const EXPORTS: usize = 22;
const BAD_OUTPUT_RATE: u32 = 11025;
include!("support/aac_eld_short.rs");

#[test]
fn third_rate_mode_requires_integer_core_geometry_and_excludes_ld_sbr() {
    use fvid_media::owned_aac::{
        NativeAacDecoder, aac_eld_synthesis::EldSynthesis, config::AudioSpecificConfig,
    };
    use fvid_media::owned_mp4::Mp4Reader;
    let m = manifest();
    let mut checked = 0;
    for c in m["refused_modes"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let asc = hex(c["asc"].as_str().unwrap());
        assert!(reader.tracks().iter().any(|t| {
            t.codec == *b"mp4a"
                && fvid_media::owned_codec_config::aac_specific_config(&t.configuration)
                    .is_ok_and(|v| v == asc.as_slice())
        }));
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let rate = c["requested_rate"].as_u64().unwrap() as u32;
        assert!(parsed.eld_downscale_factor(rate).is_none());
        assert!(parsed.resolve_output_rate(rate).is_err());
        assert!(
            NativeAacDecoder::new_with_output_rate(&asc, rate)
                .err()
                .unwrap()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        assert!(NativeAacDecoder::new(&asc).is_ok());
        checked += 1;
    }
    assert_eq!(checked, 4);
    assert!(EldSynthesis::new_downscaled(512, 3).is_err());
    assert!(EldSynthesis::new_downscaled(480, 0).is_err());
    let c = &m["cases"][0];
    let asc = hex(c["asc"].as_str().unwrap());
    let parsed = AudioSpecificConfig::parse(&asc).unwrap();
    assert_eq!(parsed.eld_core_downscale_factor(8000), Some(3));
    assert!(parsed.eld_core_downscale_factor(16000).is_none());
}

#[test]
fn third_fixtures_preserve_source_aus_and_have_nonzero_pcm() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for (corpus, protected) in [("aac-eld", false), ("aac-eld-esc", true)] {
        let parent = root().join("..").join(corpus);
        let source: serde_json::Value =
            serde_json::from_slice(&std::fs::read(parent.join("manifest.json")).unwrap()).unwrap();
        let source_blob = std::fs::read(parent.join("packets.bin")).unwrap();
        let mut count = 0;
        for c in m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["protected"].as_bool().unwrap_or(false) == protected)
        {
            let name = c["name"].as_str().unwrap();
            let original_name = name
                .strip_prefix("ep3-")
                .unwrap_or(name)
                .strip_suffix("-ds3")
                .unwrap();
            let original = source["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == original_name)
                .unwrap();
            let rows = c["frames"].as_array().unwrap();
            let original_rows = original["frames"].as_array().unwrap();
            assert_eq!(rows.len(), original_rows.len());
            for (a, b) in rows.iter().zip(original_rows) {
                let ao = a["offset"].as_u64().unwrap() as usize;
                let bo = b["offset"].as_u64().unwrap() as usize;
                let an = a["bytes"].as_u64().unwrap() as usize;
                let bn = b["bytes"].as_u64().unwrap() as usize;
                assert_eq!(&blob[ao..ao + an], &source_blob[bo..bo + bn]);
            }
            let gold = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
            assert!(
                gold.chunks_exact(4)
                    .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 1e-10)
            );
            count += 1;
        }
        assert_eq!(count, 22);
    }
}
