use fvid_media::owned_aac::{
    aac_ld_mps_config::{Config, ExtensionValue},
    bits::BitReader,
};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-config")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn number(v: &Value) -> u8 {
    v.as_u64().unwrap().try_into().unwrap()
}
fn numbers(v: &Value) -> Vec<u8> {
    v.as_array().unwrap().iter().map(number).collect()
}
fn verify(c: &Config, row: &Value) {
    let e = &row["expected"];
    let h: Vec<u8> = e["head"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, v)| if i == 1 { 0 } else { number(v) })
        .collect();
    assert_eq!(c.sample_rate, row["rate"].as_u64().unwrap() as u32);
    assert_eq!(c.frame_slots, h[2] + 1);
    assert_eq!(
        c.parameter_bands,
        [0, 23, 15, 12, 9, 7, 5, 4][h[3] as usize]
    );
    assert_eq!(c.tree.code, h[4]);
    assert_eq!(c.quantization_mode, h[5]);
    assert_eq!(c.one_icc, if h[4] == 7 { None } else { Some(h[6] != 0) });
    assert_eq!(c.arbitrary_downmix, number(&row["raw_downmix"]) != 0);
    assert_eq!(c.surround_gain, if h[4] == 7 { None } else { Some(h[8]) });
    assert_eq!(c.lfe_gain, if h[4] == 7 { None } else { Some(h[9]) });
    assert_eq!(c.downmix_gain, h[10]);
    assert_eq!(
        c.matrix_mode,
        if h[4] == 7 { None } else { Some(h[11] != 0) }
    );
    assert_eq!(c.temporal_shape, h[12]);
    assert_eq!(c.decorrelation, h[13]);
    assert_eq!(c.tree.input_channels, h[19]);
    assert_eq!(c.tree.output_channels, h[20]);
    assert_eq!(c.tree.ott_lfe.len(), h[21] as usize);
    assert_eq!(c.tree.ttt_boxes, h[22]);
    let ott = numbers(&e["ott"]);
    for (i, &lfe) in c.tree.ott_lfe.iter().enumerate() {
        assert_eq!(c.ott_bands[i], if lfe { ott[i] } else { c.parameter_bands });
    }
    let ttt = e["ttt"].as_array().unwrap();
    assert_eq!(c.ttt.len(), ttt.len());
    for (a, b) in c.ttt.iter().zip(ttt) {
        let b = numbers(b);
        assert_eq!(a.low_mode, b[1]);
        assert_eq!(a.high, if b[0] != 0 { Some((b[2], b[3])) } else { None });
    }
    let hrtf = e["hrtf"].as_array().unwrap();
    assert_eq!(c.hrtf.is_some(), !hrtf.is_empty());
    if let Some(a) = &c.hrtf {
        assert_eq!(a.asymmetric, h[17] != 0);
        assert_eq!(a.channels.len(), 5);
        assert_eq!(a.bands, [0, 23, 15, 12, 9, 7, 5, 4][h[16] as usize]);
        for (a, b) in a.channels.iter().zip(hrtf) {
            assert_eq!(a.left, numbers(&b[0]));
            assert_eq!(a.right, numbers(&b[1]));
            assert_eq!(
                a.phase,
                if number(&b[2]) != 0 {
                    Some(numbers(&b[3]))
                } else {
                    None
                }
            );
        }
    }
    assert_eq!(
        c.extensions.iter().map(|x| x.kind).collect::<Vec<_>>(),
        numbers(&e["extensions"])
    );
    for ext in &c.extensions {
        match &ext.value {
            ExtensionValue::Residual {
                rate,
                frames_code,
                bands,
            } => {
                let b = &e["residual"];
                let rates = [
                    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025,
                    8000, 7350,
                ];
                assert_eq!(*rate, rates[number(&b[0]) as usize]);
                assert_eq!(*frames_code, number(&b[1]));
                let present = numbers(&b[2]);
                let values = numbers(&b[3]);
                assert_eq!(
                    *bands,
                    present
                        .iter()
                        .zip(values)
                        .map(|(p, v)| if *p == 0 { None } else { Some(v) })
                        .collect::<Vec<_>>()
                );
            }
            ExtensionValue::DownmixResidual {
                rate,
                frames_code,
                bands,
            } => {
                let b = numbers(&e["dmx_residual"]);
                let rates = [
                    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025,
                    8000, 7350,
                ];
                assert_eq!(*rate, rates[b[0] as usize]);
                assert_eq!(*frames_code, b[1]);
                assert_eq!(*bands, b[2]);
            }
            ExtensionValue::ArbitraryTree {
                nodes,
                ott,
                positions,
            } => {
                let b = &e["arbitrary"];
                assert_eq!(ott.len(), number(&b[0]) as usize);
                assert_eq!(positions.len(), number(&b[1]) as usize);
                assert_eq!(*positions, numbers(&b[2]));
                assert_eq!(nodes.len(), c.tree.output_channels as usize);
                assert_eq!(nodes[0], [true, false, false]);
                assert!(nodes[1..].iter().all(|x| x == &[false]));
                for (i, o) in ott.iter().enumerate() {
                    assert_eq!(o.default_cld, number(&b[3][i]) != 0);
                    assert_eq!(o.lfe, number(&b[4][i]) != 0);
                    assert_eq!(o.bands, number(&b[5][i]));
                }
            }
            ExtensionValue::Unknown => assert!(ext.kind > 2),
        }
    }
}
fn offset(bytes: &[u8], at: usize) -> Vec<u8> {
    let mut out = vec![0; (at + bytes.len() * 8 + 8).div_ceil(8)];
    for i in 0..bytes.len() * 8 {
        out[(at + i) / 8] |= ((bytes[i / 8] >> (7 - i % 8)) & 1) << (7 - (at + i) % 8);
    }
    out
}
#[test]
fn standalone_spatial_metadata_matches_own_fields_at_every_bit_offset() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 336);
    for row in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(row["file"].as_str().unwrap())).unwrap();
        let config = Config::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", row["name"]));
        verify(&config, row);
        for at in 0..8 {
            let data = offset(&bytes, at);
            let mut bits = BitReader::new(&data);
            bits.skip(at).unwrap();
            assert_eq!(Config::read(&mut bits, bytes.len()).unwrap(), config);
            assert_eq!(bits.position(), at + bytes.len() * 8);
            assert_eq!(bits.read(8).unwrap(), 0);
        }
    }
}
fn set(bytes: &mut [u8], at: usize, width: usize, value: u32) {
    for i in 0..width {
        let bit = 1 << (7 - (at + i) % 8);
        let b = &mut bytes[(at + i) / 8];
        *b = (*b & !bit) | (((value >> (width - i - 1)) as u8 & 1) * bit);
    }
}
#[test]
fn malformed_fields_and_short_children_never_consume_the_next_container() {
    let basic = std::fs::read(root().join("tree7-res1-v0.bin")).unwrap();
    let complex = std::fs::read(root().join("tree0-res1-v1.bin")).unwrap();
    let extensions = std::fs::read(root().join("tree7-res1-v5.bin")).unwrap();
    let mut bad = Vec::new();
    for (at, width, value) in [
        (0, 4, 13),
        (9, 3, 0),
        (12, 4, 8),
        (16, 2, 3),
        (22, 2, 3),
        (24, 2, 3),
    ] {
        let mut b = basic.clone();
        set(&mut b, at, width, value);
        bad.push(b);
    }
    for (at, width, value) in [(40, 2, 3), (42, 3, 0)] {
        let mut b = complex.clone();
        set(&mut b, at, width, value);
        bad.push(b);
    }
    let mut zero_rate = std::fs::read(root().join("tree1-res1-v5.bin")).unwrap();
    set(&mut zero_rate, 4, 24, 0);
    bad.push(zero_rate);
    let multi = std::fs::read(root().join("tree0-res1-v0.bin")).unwrap();
    for (at, width, value) in [(20, 3, 7), (23, 3, 7), (35, 5, 31)] {
        let mut b = multi.clone();
        set(&mut b, at, width, value);
        bad.push(b);
    }
    let dual = std::fs::read(root().join("tree2-res1-v3.bin")).unwrap();
    for (at, width, value) in [(41, 3, 7), (44, 3, 7), (47, 5, 31)] {
        let mut b = dual.clone();
        set(&mut b, at, width, value);
        bad.push(b);
    }
    let mut envelope = std::fs::read(root().join("tree7-res1-v2.bin")).unwrap();
    set(&mut envelope, 26, 1, 1);
    bad.push(envelope);
    let mut unfinished = std::fs::read(root().join("tree7-res1-v3.bin")).unwrap();
    unfinished[5..].fill(0xff);
    bad.push(unfinished);
    let mut b = extensions.clone();
    set(&mut b, 36, 4, 0);
    bad.push(b); // zero-length residual cannot read the following records
    for data in bad {
        assert!(Config::parse(&data).is_err());
        for at in 0..8 {
            let shifted = offset(&data, at);
            let mut bits = BitReader::new(&shifted);
            bits.skip(at).unwrap();
            assert!(Config::read(&mut bits, data.len()).is_err());
            assert_eq!(bits.position(), at);
        }
    }
    let mut extent = BitReader::new(&basic);
    assert!(Config::read(&mut extent, usize::MAX).is_err());
    assert_eq!(extent.position(), 0);
    for at in 0..8 {
        let shifted = offset(&complex, at);
        for available in 0..complex.len() * 8 {
            let mut bits = BitReader::with_bit_len(&shifted, at + available).unwrap();
            bits.skip(at).unwrap();
            assert!(Config::read(&mut bits, complex.len()).is_err());
            assert_eq!(bits.position(), at);
        }
    }
}
#[test]
fn eld_spatial_movies_reach_the_specific_renderer_refusal() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    for row in m["movies"].as_array().unwrap() {
        let asc = std::fs::read(root().join(row["asc"].as_str().unwrap())).unwrap();
        let error = match NativeAacDecoder::new(&asc) {
            Ok(_) => panic!("spatial reconstruction must not silently output the downmix"),
            Err(e) => e.to_string(),
        };
        assert!(
            error.contains("LD-MPS spatial reconstruction"),
            "{}: {error}",
            row["name"]
        );
    }
}
#[test]
fn eld_spatial_configuration_metadata_is_accepted_without_claiming_movie_playback() {
    use fvid_media::owned_aac::config::AudioSpecificConfig;
    let m = manifest();
    for row in m["movies"].as_array().unwrap() {
        let asc = std::fs::read(root().join(row["asc"].as_str().unwrap())).unwrap();
        let payload = std::fs::read(root().join(row["config"].as_str().unwrap())).unwrap();
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let root_parsed = fvid::codec::config::AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(root_parsed.ld_mps, parsed.ld_mps);
        let spatial = parsed.ld_mps.as_ref().unwrap();
        assert_eq!(*spatial, Config::parse(&payload).unwrap());
        assert_eq!(spatial.tree.input_channels, number(&row["input_channels"]));
        assert_eq!(
            spatial.tree.output_channels,
            number(&row["output_channels"])
        );
        let movie = std::fs::read(root().join(row["video"].as_str().unwrap())).unwrap();
        let mut public = Vec::new();
        let public_error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut public)
            .unwrap_err()
            .to_string();
        assert!(
            public_error.contains("LD-MPS"),
            "{}: {public_error}",
            row["name"]
        );
        assert!(public.is_empty());
        let mut pcm = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(movie),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("LD-MPS"), "{}: {error}", row["name"]);
        assert!(pcm.is_empty());
    }
}
