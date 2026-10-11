use fvid::codec::hevc_vps::{LayeredVps, Vps};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn complete_layered_vps_metadata_nal_and_rbsp_boundaries() {
    let cases: serde_json::Value = serde_json::from_slice(&data("hevc-layered-vps.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let result = LayeredVps::parse(&bytes, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            let e = result.unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "{name}: {e}");
            assert!(e.to_string().contains(error), "{name}: {e}");
            continue;
        }
        let p = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let extension = p.extension.as_ref().unwrap();
        assert_eq!(p.vps.layers.base_internal, c["internal"].as_bool().unwrap());
        assert_eq!(p.vps.layers.sets, vec![1, 32905]);
        assert_eq!(p.vps.ordering.len(), 3);
        assert_eq!(extension.graph.layer_ids, vec![0, 3, 7, 15]);
        assert_eq!(extension.dpb.sets[1].as_ref().unwrap().len(), 3);
        assert!(p.vps.poc_lsb_aligned().unwrap());
        assert_eq!(p.future_extension, c["future"].as_bool().unwrap());
        assert_eq!(p.vui.is_some(), c["vui"].as_bool().unwrap());
        assert_eq!(p.bsp.is_some(), c["bsp"].as_bool().unwrap());
        assert_eq!(p.vps.timing.is_some(), p.bsp.is_some());
        if let Some(bsp) = p.bsp.as_ref() {
            assert_eq!(bsp.additional_hrd.len(), 1);
            assert_eq!(
                bsp.sets[1].as_ref().unwrap().len(),
                c["schemes"].as_u64().unwrap() as usize
            );
            if c["base_hrd"].as_bool().unwrap() {
                assert_eq!(p.vps.layer_hrd.len(), 1);
                assert_eq!(
                    bsp.additional_hrd[0].common,
                    p.vps.layer_hrd[0].parameters.common
                );
                for rows in &bsp.sets[1].as_ref().unwrap()[0].schedules {
                    for (i, row) in rows.iter().enumerate() {
                        for entry in row {
                            assert_eq!(entry.hrd_index, i as u16);
                            assert_eq!(entry.cpb_index, 0);
                        }
                    }
                }
            }
        }
        assert!(
            matches!(
                Vps::parse(&bytes, 1 << 20),
                Err(fvid::Error::Unsupported(_))
            ),
            "metadata must not admit multilayer playback"
        );
        assert!(LayeredVps::parse(&bytes, bytes.len() + 256).is_err());
        for end in 0..bytes.len() {
            let shortened = LayeredVps::parse(&bytes[..end], 1 << 20);
            if p.future_extension {
                // An opaque future tail has no coded length: a shorter tail
                // can itself end with valid trailing bits. Preserve semantics.
                if let Ok(short) = shortened {
                    assert!(short.future_extension);
                    assert_eq!(short.vps, p.vps);
                    assert_eq!(short.extension, p.extension);
                }
            } else {
                assert!(shortened.is_err(), "{name}: byte {end}");
            }
        }
    }
}
