use fvid_media::owned_aac::{
    NativeAacDecoder,
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_session::{Session, Stamp},
    aac_ld_mps_transport::Assembler,
    aac_pce::read_eld_spatial_extensions,
    bits::BitReader,
};
fn folder(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn source(name: &str) -> Vec<u8> {
    std::fs::read(folder("aac-eld-mps-core").join(name)).unwrap()
}
#[test]
fn mixed_er_tail_records_preserve_spatial_pcm_and_following_extensions() {
    let root = folder("aac-eld-spatial-tail");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(root.join("packets.bin")).unwrap();
    let mut reference = std::collections::BTreeMap::new();
    let mut successes = 0;
    let mut failures = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let tree = case["tree"].as_u64().unwrap() as usize;
        let config = Config::parse(&source(&format!("tree{tree}.ssc"))).unwrap();
        let mut core = NativeAacDecoder::new(&source(&format!("tree{tree}.asc"))).unwrap();
        let mut session = Session::new(&config, Mode::Complex).unwrap();
        let mut assembler = Assembler::default();
        let mut output = Vec::new();
        for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = frame["offset"].as_u64().unwrap() as usize;
            let n = frame["bytes"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + n];
            let start = frame["core_bits"].as_u64().unwrap() as usize;
            let mut bits = BitReader::new(packet);
            bits.skip(start).unwrap();
            let saved = assembler.clone();
            if !case["error"].is_null() && index == 1 {
                let error = read_eld_spatial_extensions(&mut bits, &mut assembler)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(case["error"].as_str().unwrap()),
                    "{}: {error}",
                    case["name"]
                );
                assert_eq!(bits.position(), start);
                assert_eq!(assembler, saved);
                failures += 1;
                break;
            }
            let messages = read_eld_spatial_extensions(&mut bits, &mut assembler).unwrap();
            assert_eq!(bits.remaining(), 0);
            let mut replay = saved;
            let mut cursor = BitReader::new(packet);
            cursor.skip(start).unwrap();
            assert_eq!(
                read_eld_spatial_extensions(&mut cursor, &mut replay).unwrap(),
                messages
            );
            assert_eq!(replay, assembler);
            let samples = core.decode(packet).unwrap();
            let channels = usize::from(config.tree.input_channels);
            let planar: Vec<Vec<f64>> = (0..channels)
                .map(|c| {
                    samples
                        .chunks_exact(channels)
                        .map(|r| f64::from(r[c]))
                        .collect()
                })
                .collect();
            output.extend(
                session
                    .submit(
                        Some((
                            &planar,
                            Stamp {
                                pts: index as i64 * 512,
                                duration: 512,
                                timescale: 48000,
                            },
                        )),
                        &messages,
                    )
                    .unwrap(),
            );
        }
        if case["error"].is_null() {
            assembler.finish().unwrap();
            session.finish().unwrap();
            assert_eq!(output.len(), 3);
            if let Some(expected) = reference.get(&tree) {
                assert_eq!(&output, expected, "{}", case["name"]);
            } else {
                reference.insert(tree, output);
            }
            successes += 1;
        }
    }
    assert_eq!((successes, failures), (38, 6));
}
#[test]
fn late_bad_drc_and_alignment_restore_pending_fragments_and_cursor() {
    let mut assembler = Assembler::default();
    let mut start = BitReader::new(&[0x92, 0xa1]);
    assert!(
        read_eld_spatial_extensions(&mut start, &mut assembler)
            .unwrap()
            .is_empty()
    );
    let saved = assembler.clone();
    for bytes in [
        vec![0x32, 0x91, 0xb2, 0xb8],
        vec![0x32, 0x91, 0xb2, 0x32, 0xb8, 0x00],
    ] {
        let mut bits = BitReader::new(&bytes);
        assert!(read_eld_spatial_extensions(&mut bits, &mut assembler).is_err());
        assert_eq!(bits.position(), 0);
        assert_eq!(assembler, saved);
    }
    // Three zero alignment bits follow a valid bounded stop, then one bad bit.
    let bytes = [0x32, 0x91, 0xb2, 0x10];
    let mut bits = BitReader::with_bit_len(&bytes, 28).unwrap();
    assert!(
        read_eld_spatial_extensions(&mut bits, &mut assembler)
            .unwrap_err()
            .to_string()
            .contains("alignment")
    );
    assert_eq!(assembler, saved);
    assert_eq!(bits.position(), 0);
    let mut valid = BitReader::new(&[0x32, 0x91, 0xb2, 0xb0, 0x00]);
    let messages = read_eld_spatial_extensions(&mut valid, &mut assembler).unwrap();
    assert_eq!(messages[0].payload, [0xa1, 0xb2]);
    assembler.finish().unwrap();
}
#[test]
fn native_mixed_tail_movies_play_or_refuse_the_specific_tail_failure() {
    let root = folder("aac-eld-spatial-tail");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let movie =
            std::fs::read(root.join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        let result = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm);
        if let Some(reason) = case["error"].as_str() {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(reason), "{}: {error}", case["name"]);
        } else {
            result.unwrap();
            assert!(!pcm.is_empty());
        }
    }
}

#[test]
fn frontend_and_owned_cursors_share_one_ancillary_assembly_history() {
    let mut assembler = Assembler::default();
    read_eld_spatial_extensions(&mut BitReader::new(&[0x92, 0xa1]), &mut assembler).unwrap();
    let mut frontend = fvid::codec::bits::BitReader::new(&[0x32, 0x91, 0xb2]);
    let result =
        fvid::codec::aac_pce::read_eld_spatial_extensions(&mut frontend, &mut assembler).unwrap();
    assert_eq!(result[0].payload, [0xa1, 0xb2]);
    assert_eq!(frontend.remaining(), 0);
    assembler.finish().unwrap();
    let before = assembler.clone();
    assert!(assembler.read_bytes(&[0x93], 9, 8).is_err());
    assert_eq!(assembler, before);
    assert!(assembler.read_bytes(&[0x93], 0, 9).is_err());
    assert_eq!(assembler, before);
}

#[test]
fn spatial_dispatch_accepts_bounded_er_records_at_every_storage_bit_offset() {
    fn shifted(bytes: &[u8], offset: usize) -> Vec<u8> {
        let mut out = vec![0; (bytes.len() * 8 + offset).div_ceil(8)];
        for i in 0..bytes.len() * 8 {
            out[(offset + i) / 8] |= ((bytes[i / 8] >> (7 - i % 8)) & 1) << (7 - (offset + i) % 8);
        }
        out
    }
    for offset in 0..8 {
        let mut assembler = Assembler::default();
        let start = shifted(&[0x92, 0xa1], offset);
        let mut bits = BitReader::with_bit_len(&start, offset + 16).unwrap();
        bits.skip(offset).unwrap();
        assert!(
            read_eld_spatial_extensions(&mut bits, &mut assembler)
                .unwrap()
                .is_empty()
        );
        let stop = shifted(&[0x32, 0x91, 0xb2, 0xb0, 0x00], offset);
        let mut frontend = fvid::codec::bits::BitReader::with_bit_len(&stop, offset + 40).unwrap();
        frontend.skip(offset).unwrap();
        let result =
            fvid::codec::aac_pce::read_eld_spatial_extensions(&mut frontend, &mut assembler)
                .unwrap();
        assert_eq!(result[0].payload, [0xa1, 0xb2]);
        assert_eq!(frontend.remaining(), 0);
        assembler.finish().unwrap();
    }
}

#[test]
fn native_mixed_tail_movies_preserve_the_complete_spatial_pcm() {
    let root = folder("aac-eld-spatial-tail");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut references = std::collections::BTreeMap::new();
    for tree in 0..8 {
        let movie = source(&format!("tree{tree}.mp4"));
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
        assert!(!pcm.is_empty());
        references.insert(tree, pcm);
    }
    for case in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
    {
        let movie =
            std::fs::read(root.join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
        assert_eq!(
            pcm,
            references[&(case["tree"].as_u64().unwrap() as usize)],
            "{}",
            case["name"]
        );
    }
}
