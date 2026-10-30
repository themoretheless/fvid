use fvid_media::owned_aac::{
    aac_ld_mps_transport::{Assembler, Extension, Kind},
    bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-transport")
}
fn shifted(data: &[u8], bits: usize, prefix: usize) -> Vec<u8> {
    let mut out = vec![0; (prefix + bits + 8).div_ceil(8)];
    for i in 0..bits {
        out[(prefix + i) / 8] |= ((data[i / 8] >> (7 - i % 8)) & 1) << (7 - (prefix + i) % 8);
    }
    for i in 0..8 {
        out[(prefix + bits + i) / 8] |= ((0x5au8 >> (7 - i)) & 1) << (7 - (prefix + bits + i) % 8);
    }
    out
}
#[test]
fn all_envelopes_lengths_fragmentation_and_offsets_preserve_payload_and_next_extension() {
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(r.join("envelopes.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 224);
    for c in m["cases"].as_array().unwrap() {
        let n = c["length"].as_u64().unwrap() as usize;
        let kind = c["kind"].as_u64().unwrap() as usize;
        let expected = (0..n)
            .map(|i| ((i * 37 + n * 3 + kind) % 251) as u8)
            .collect::<Vec<_>>();
        for prefix in 0..8 {
            let mut decoder = Assembler::default();
            let fresh = decoder.clone();
            for (fno, f) in c["fragments"].as_array().unwrap().iter().enumerate() {
                let at = f["offset"].as_u64().unwrap() as usize;
                let nb = f["bytes"].as_u64().unwrap() as usize;
                let nbits = f["bits"].as_u64().unwrap() as usize;
                let input = shifted(&blob[at..at + nb], nbits, prefix);
                let mut reader = BitReader::new(&input);
                reader.skip(prefix).unwrap();
                let saved = decoder.clone();
                let end = prefix + nbits + if c["wrapped"] == true { 8 } else { 0 };
                let result = decoder.read(&mut reader, end).unwrap();
                assert_eq!(reader.position(), prefix + nbits);
                assert_eq!(reader.read(8).unwrap(), 0x5a);
                let mut replay = saved;
                let mut b = BitReader::new(&input);
                b.skip(prefix).unwrap();
                assert_eq!(replay.read(&mut b, end).unwrap(), result);
                assert_eq!(replay, decoder);
                if f["complete"] == true {
                    let message = result.unwrap();
                    assert_eq!(
                        message.kind,
                        if kind == 0 {
                            Kind::Frame
                        } else {
                            Kind::HeaderAndFrame
                        }
                    );
                    assert_eq!(
                        message.extension,
                        if c["tag"] == 9 {
                            Extension::LowDelay
                        } else {
                            Extension::Surround
                        }
                    );
                    assert_eq!(message.payload, expected, "fragment {fno}, case {c}");
                } else {
                    assert!(result.is_none());
                }
            }
            decoder.reset();
            assert_eq!(decoder, fresh);
        }
    }
}
#[test]
fn invalid_envelopes_and_fragment_order_preserve_pending_message_and_cursor() {
    let mut a = Assembler::default();
    let mut start = BitReader::new(&[0x92, 0xa1]);
    assert!(a.read(&mut start, 16).unwrap().is_none());
    let saved = a.clone();
    assert!(a.finish().unwrap_err().to_string().contains("unfinished"));
    assert_eq!(a, saved);
    for (bytes, end, reason) in [
        (vec![0x92, 0xb2], 16, "start while"),
        (vec![0x9b, 0xb2], 16, "reserved"),
        (vec![0x31, 0x33], 16, "nested"),
        (vec![0x32, 0x93], 16, "exceeds"),
        (vec![0x93], 7, "envelope"),
        (vec![0x93, 0xff], 15, "non-byte"),
    ] {
        let mut bits = BitReader::new(&bytes);
        let error = a.read(&mut bits, end).unwrap_err().to_string();
        assert!(error.contains(reason), "{error}");
        assert_eq!(bits.position(), 0);
        assert_eq!(a, saved);
    }
    let mut stop = BitReader::new(&[0x91, 0xc3]);
    assert_eq!(
        a.read(&mut stop, 16).unwrap().unwrap().payload,
        [0xa1, 0xc3]
    );
    a.finish().unwrap();
    let mut orphan = BitReader::new(&[0x90]);
    let saved = a.clone();
    assert!(
        a.read(&mut orphan, 8)
            .unwrap_err()
            .to_string()
            .contains("without start")
    );
    assert_eq!(a, saved);
    assert_eq!(orphan.position(), 0);
}

#[test]
fn reassembled_spatial_frames_feed_the_pcm_chain_without_changing_saved_audio() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm::Decoder,
        aac_sbr_qmf::Complex,
    };
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let envelopes = std::fs::read(r.join("envelopes.bin")).unwrap();
    let p = r.parent().unwrap().join("aac-ld-mps-pcm");
    let pcm: Value =
        serde_json::from_slice(&std::fs::read(p.join("manifest.json")).unwrap()).unwrap();
    let expected = std::fs::read(p.join("expected.bin")).unwrap();
    assert_eq!(m["programmes"].as_array().unwrap().len(), 224);
    let mut samples = 0;
    for programme in m["programmes"].as_array().unwrap() {
        let c = &pcm["cases"][programme["pcm_case"].as_u64().unwrap() as usize];
        let bytes = c["config"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let config = Config::parse(&bytes).unwrap();
        let bins = c["bins"].as_u64().unwrap() as usize;
        let slots = config.frame_slots as usize;
        let mut decoder = Decoder::new(
            &config,
            if c["mode"] == 0 {
                Mode::Real
            } else {
                Mode::Complex
            },
        )
        .unwrap();
        let mut transport = Assembler::default();
        for (fi, f) in programme["frames"].as_array().unwrap().iter().enumerate() {
            let mut message = None;
            for fragment in f["fragments"].as_array().unwrap() {
                let offset = fragment["offset"].as_u64().unwrap() as usize;
                let n = fragment["bytes"].as_u64().unwrap() as usize;
                let bits = fragment["bits"].as_u64().unwrap() as usize;
                let mut reader = BitReader::new(&envelopes[offset..offset + n]);
                message = transport.read(&mut reader, bits).unwrap();
                assert_eq!(reader.position(), bits);
            }
            let message = message.unwrap();
            assert_eq!(message.kind, Kind::Frame);
            let input = (0..config.tree.input_channels as usize)
                .map(|ch| {
                    (0..slots * bins)
                        .map(|i| {
                            if fi == 2 {
                                0.0
                            } else {
                                (((fi * 11 + i * 7 + ch * 3) % 29) as f64 - 14.0) / 1024.0
                            }
                        })
                        .collect()
                })
                .collect::<Vec<_>>();
            let residuals = (0..config.tree.ott_lfe.len() + config.tree.ttt_boxes as usize)
                .map(|ch| {
                    Some(
                        (0..slots)
                            .map(|t| {
                                (0..bins)
                                    .map(|q| {
                                        if fi == 2 {
                                            Complex::default()
                                        } else {
                                            Complex {
                                                re: (((fi * 11 + t * 7 + ch * 3 + q * 5) % 29)
                                                    as f64
                                                    - 14.0)
                                                    / 512.0,
                                                im: (((fi * 7 + t * 5 + ch * 11 + q * 3) % 23)
                                                    as f64
                                                    - 11.0)
                                                    / 512.0,
                                            }
                                        }
                                    })
                                    .collect()
                            })
                            .collect(),
                    )
                })
                .collect::<Vec<_>>();
            let mut reader = BitReader::new(&message.payload);
            let output = decoder.decode(&mut reader, &input, &residuals).unwrap();
            assert_eq!(reader.position(), message.payload.len() * 8);
            let offset = c["frames"][fi]["offset"].as_u64().unwrap() as usize;
            for (i, &actual) in output.samples.iter().enumerate() {
                let at = offset + i * 4;
                let wanted = f32::from_le_bytes(expected[at..at + 4].try_into().unwrap()) as f64;
                assert!((actual - wanted).abs() < 5e-6);
                samples += 1;
            }
        }
    }
    assert_eq!(samples, 1677312);
}
