#![cfg(feature = "player")]
use std::{io::Cursor, path::Path};
#[test]
fn matching_in_band_pce_with_indexed_asc_preserves_independent_pcm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-indexed-pce.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let video = std::fs::read(root.join(c["video"]["file"].as_str().unwrap())).unwrap();
        let mut actual = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&video),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        let expected = std::fs::read(root.join(c["pcm_file"].as_str().unwrap())).unwrap();
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (i, (a, b)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 2e-7, "{name}/{i}: {a} vs {b}");
        }
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        let blob = std::fs::read(root.join("aac-indexed-pce-packets.bin")).unwrap();
        if let Some(tag) = c["tag"].as_u64() {
            let first = &c["frames"][0];
            let offset = first["offset"].as_u64().unwrap() as usize;
            let length = first["bytes"].as_u64().unwrap() as usize;
            let mut bits = fvid::codec::bits::BitReader::new(&blob[offset..offset + length]);
            let kind = if c["channels"] == 2 { 1 } else { 0 };
            if c["late"].as_bool().unwrap() {
                assert_eq!(bits.read(3).unwrap(), kind, "{name}: no initial PCE");
                assert_eq!(bits.read(4).unwrap(), tag as u32);
                let last = c["frames"].as_array().unwrap().last().unwrap();
                let offset = last["offset"].as_u64().unwrap() as usize;
                let length = last["bytes"].as_u64().unwrap() as usize;
                bits = fvid::codec::bits::BitReader::new(&blob[offset..offset + length]);
            }
            assert_eq!(bits.read(3).unwrap(), 5, "{name}: confirming PCE");
            let program = fvid::codec::aac_pce::ProgramConfig::read(&mut bits, 0).unwrap();
            assert_eq!(program.elements[0].tag, tag as u8);
            assert_eq!(bits.read(3).unwrap(), kind);
            assert_eq!(bits.read(4).unwrap(), tag as u32);
        }
        assert!(
            expected
                .chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()) != 0.0)
        );
        let mut decoder = fvid::codec::aac_native::NativeAacDecoder::new(&asc).unwrap();
        let mut decoded = Vec::new();
        for frame in c["frames"].as_array().unwrap() {
            // A truncated PCE must not commit a partial program or history.
            assert!(decoder.decode(&[0xa0]).is_err());
            let offset = frame["offset"].as_u64().unwrap() as usize;
            let length = frame["bytes"].as_u64().unwrap() as usize;
            decoded.extend(decoder.decode(&blob[offset..offset + length]).unwrap());
        }
        while let Some(frame) = decoder.finish().unwrap() {
            decoded.extend(frame.samples);
        }
        assert_eq!(decoded.len() * 4, expected.len(), "{name} frontend");
        for (a, b) in decoded.iter().zip(expected.chunks_exact(4)) {
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!((*a - b).abs() < 2e-7, "{name} frontend: {a} vs {b}");
        }
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
        assert!(reader.read_frame_raw().unwrap().is_some());
    }
}

#[test]
fn indexed_asc_rejects_in_band_pce_profile_or_rate_changes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-indexed-pce.json")).unwrap()).unwrap();
    let blob = std::fs::read(root.join("aac-indexed-pce-packets.bin")).unwrap();
    for c in manifest["invalid"].as_array().unwrap() {
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        let frame = &c["frames"][0];
        let offset = frame["offset"].as_u64().unwrap() as usize;
        let length = frame["bytes"].as_u64().unwrap() as usize;
        let packet = &blob[offset..offset + length];
        let video = std::fs::read(root.join(c["video"]["file"].as_str().unwrap())).unwrap();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(video),
            &mut Vec::new(),
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("AAC in-band PCE changed the configured layout"),
            "{}: {error}",
            c["name"]
        );
        let mut root_decoder = fvid::codec::aac_native::NativeAacDecoder::new(&asc).unwrap();
        let mut library = fvid_media::owned_aac::NativeAacDecoder::new(&asc).unwrap();
        for error in [
            root_decoder.decode(packet).unwrap_err().to_string(),
            library.decode(packet).unwrap_err().to_string(),
        ] {
            assert!(
                error.contains("AAC in-band PCE changed the configured layout"),
                "{}: {error}",
                c["name"]
            );
        }
    }
}
