use std::{io::Cursor, path::Path};

#[test]
fn owned_library_and_frontend_parse_identical_sps_and_pps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut parameter_sets = 0;
    for name in [
        "video.mp4",
        "short/avc-bframes.mp4",
        "playback-errors/avc-slice-lists-temporal.mp4",
        "playback-errors/avc-bypass-main10.mp4",
    ] {
        let input = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        for track in input.tracks() {
            if !matches!(&track.codec, b"avc1" | b"avc3") {
                continue;
            }
            let config =
                fvid_media::owned_codec_config::AvcConfig::parse(&track.configuration).unwrap();
            for nal in config.sps {
                let owned = fvid_media::owned_avc::Sps::parse(nal).unwrap();
                let frontend = fvid::codec::avc::Sps::parse(nal).unwrap();
                assert_eq!(format!("{owned:?}"), format!("{frontend:?}"), "{name}");
                assert_eq!(owned.display_dimensions(), frontend.display_dimensions());
                for &pps in &config.pps {
                    let a = fvid_media::owned_avc::Pps::parse(pps, &owned).unwrap();
                    let b = fvid::codec::avc::Pps::parse(pps, &frontend).unwrap();
                    assert_eq!(format!("{a:?}"), format!("{b:?}"), "{name}");
                }
                parameter_sets += 1;
            }
        }
    }
    assert!(parameter_sets >= 4);
}

#[test]
fn profile_metadata_uses_sps_and_refuses_ambiguous_declarations() {
    use fvid_media::{owned_avc::configuration_profile_level, owned_codec_config::AvcConfig};
    let input = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/avc-slice-lists-temporal.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let bytes = &input.tracks()[0].configuration;
    let expected = configuration_profile_level(bytes).unwrap();
    let mut stale_header = bytes.clone();
    stale_header[3] = expected.1.wrapping_add(1);
    assert_eq!(configuration_profile_level(&stale_header), Some(expected));
    let config = AvcConfig::parse(bytes).unwrap();
    let first = config.sps[0];
    let mut different_level = first.to_vec();
    different_level[3] = expected.1.wrapping_add(1);
    assert_ne!(
        fvid_media::owned_avc::Sps::parse(first).unwrap().level,
        fvid_media::owned_avc::Sps::parse(&different_level)
            .unwrap()
            .level
    );
    let mut heterogeneous = vec![
        1,
        config.profile,
        config.compatibility,
        config.level,
        0xfc | (config.length_size - 1),
        0xe2,
    ];
    for sps in [first, different_level.as_slice()] {
        heterogeneous.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        heterogeneous.extend_from_slice(sps);
    }
    heterogeneous.push(config.pps.len() as u8);
    for pps in config.pps {
        heterogeneous.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        heterogeneous.extend_from_slice(pps);
    }
    AvcConfig::parse(&heterogeneous).unwrap();
    assert_eq!(configuration_profile_level(&heterogeneous), None);
    let truncated_sps = [
        1, 100, 0, 31, 0xff, 0xe1, 0, 4, 0x67, 100, 0, 31, 1, 0, 1, 0x68,
    ];
    AvcConfig::parse(&truncated_sps).unwrap();
    assert_eq!(configuration_profile_level(&truncated_sps), None);
}
