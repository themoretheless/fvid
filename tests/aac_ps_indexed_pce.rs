#![cfg(feature = "player")]
use std::path::Path;
#[test]
fn indexed_mono_asc_accepts_confirming_ps_pce_and_preserves_stereo_pcm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-ps-indexed-pce.json")).unwrap())
            .unwrap();
    let blob = std::fs::read(root.join("aac-ps-indexed-pce-packets.bin")).unwrap();
    let gold = std::fs::read(root.join("aac-ps-absence-pcm.bin")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        let rate = c["container_rate"].as_u64().unwrap() as u32;
        let mut decoder =
            fvid::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(&asc, rate)
                .unwrap();
        let mut probe = fvid::codec::aac_ps_native::InBandPsProbe::new(&asc, rate).unwrap();
        let mut actual = Vec::new();
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let offset = row["offset"].as_u64().unwrap() as usize;
            let length = row["bytes"].as_u64().unwrap() as usize;
            let wire = &blob[offset..offset + length];
            assert!(probe.read(&[0xa0]).is_err());
            assert!(decoder.decode(&[0xa0]).is_err());
            assert_eq!(probe.read(wire).unwrap(), index > 0);
            let saved = decoder.checkpoint();
            let frame = decoder.decode(wire).unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(frame, decoder.decode(wire).unwrap());
            if let Some(frame) = frame {
                actual.extend(frame.pcm);
            }
        }
        if let Some(frame) = decoder.finish().unwrap() {
            actual.extend(frame.pcm);
        }
        let video = std::fs::read(root.join(c["video"]["file"].as_str().unwrap())).unwrap();
        let mut container_pcm = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&video),
            &mut container_pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let container_pcm: Vec<f32> = container_pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(container_pcm, actual, "{} MP4", c["name"]);
        let mut picture =
            fvid::playback_native::NativeReader::software(std::io::Cursor::new(video), 32 << 20)
                .unwrap();
        assert!(picture.read_frame_raw().unwrap().is_some());
        assert_eq!(actual.len(), c["pcm"][0][1].as_u64().unwrap() as usize * 2);
        for (index, value) in actual.iter().enumerate() {
            let offset = c["pcm"][index % 2][0].as_u64().unwrap() as usize + index / 2 * 8;
            let expected = f64::from_le_bytes(gold[offset..offset + 8].try_into().unwrap()) as f32;
            assert!(
                (value - expected).abs() <= 2. * f32::EPSILON * expected.abs() + 2e-16,
                "{} sample{index}: {value} vs {expected}",
                c["name"]
            );
        }
    }
}

#[test]
fn indexed_ps_pce_profile_and_rate_mismatches_refuse_atomically() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-ps-indexed-pce.json")).unwrap())
            .unwrap();
    let blob = std::fs::read(root.join("aac-ps-indexed-pce-packets.bin")).unwrap();
    for c in manifest["invalid"].as_array().unwrap() {
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        let rate = c["container_rate"].as_u64().unwrap() as u32;
        let row = &c["frames"][0];
        let offset = row["offset"].as_u64().unwrap() as usize;
        let length = row["bytes"].as_u64().unwrap() as usize;
        let wire = &blob[offset..offset + length];
        let mut decoder =
            fvid::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(&asc, rate)
                .unwrap();
        let mut probe = fvid::codec::aac_ps_native::InBandPsProbe::new(&asc, rate).unwrap();
        for error in [
            decoder.decode(wire).unwrap_err().to_string(),
            probe.read(wire).unwrap_err().to_string(),
        ] {
            assert!(
                error.contains("PS AAC in-band PCE changed the configured layout"),
                "{}: {error}",
                c["name"]
            );
        }
        let control = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == c["control"])
            .unwrap();
        let row = &control["frames"][0];
        let offset = row["offset"].as_u64().unwrap() as usize;
        let length = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(probe.read(&blob[offset..offset + length]).unwrap(), false);
        assert!(
            decoder
                .decode(&blob[offset..offset + length])
                .unwrap()
                .is_none()
        );
        let video = std::fs::read(root.join(c["video"]["file"].as_str().unwrap())).unwrap();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(video),
            &mut Vec::new(),
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("in-band PCE changed the configured layout"),
            "{}: {error}",
            c["name"]
        );
    }
}
