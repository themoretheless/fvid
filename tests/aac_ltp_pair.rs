use fvid_media::owned_aac::{
    aac_bands::BandTables, aac_ltp_channel::LtpChannel, aac_pair::ChannelPair, bits::BitReader,
    config::AacConfig,
};
use serde_json::Value;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn ltp_pair_packets_match_scalar_and_qualified_external_pcm() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
    let blob = bytes("aac-ltp-pair-packets.bin");
    assert_eq!(m["cases"].as_array().unwrap().len(), 4);
    let config = AacConfig {
        object_type: 4,
        sample_rate: 24000,
        channels: 2,
        channel_configuration: 2,
        frame_samples: 1024,
        core_coder_delay: None,
    };
    let offsets = BandTables::new(24000, 1024).unwrap().long;
    for case in m["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let gold = bytes(&format!("aac-ltp-pair-{name}-scalar-reference.f32le"));
        let external = bytes(&format!("aac-ltp-pair-{name}-external-reference.f32le"));
        let mut external_peaks = [0f64; 2];
        assert_eq!(gold.len(), 12288 * 2 * 4);
        let mut states = [
            LtpChannel::new(1024).unwrap(),
            LtpChannel::new(1024).unwrap(),
        ];
        for (frame, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut b = BitReader::new(raw);
            assert_eq!(b.read(3).unwrap(), 1);
            assert_eq!(b.read(4).unwrap(), 0);
            let (pair, data) = ChannelPair::read_ltp(&mut b, &config).unwrap();
            assert_eq!(b.read(3).unwrap(), 7);
            for ch in 0..2 {
                assert_eq!(data[ch].is_some(), row["active"][ch].as_bool().unwrap());
            }
            let (left, right) = pair.ordinary_spectra(&config).unwrap();
            let channels = [&pair.left, &pair.right];
            let mut pcm = Vec::new();
            for (ch, spectrum) in [left, right].into_iter().enumerate() {
                let saved = states[ch].checkpoint();
                let channel = channels[ch];
                let first = states[ch]
                    .process(
                        spectrum.clone(),
                        data[ch].as_ref(),
                        channel.info.sequence,
                        channel.info.shape,
                        offsets,
                        2,
                        channel.tns.as_ref(),
                    )
                    .unwrap();
                states[ch].restore(&saved).unwrap();
                assert_eq!(
                    states[ch]
                        .process(
                            spectrum,
                            data[ch].as_ref(),
                            channel.info.sequence,
                            channel.info.shape,
                            offsets,
                            2,
                            channel.tns.as_ref()
                        )
                        .unwrap(),
                    first
                );
                pcm.push(first);
            }
            for i in 0..1024 {
                for ch in 0..2 {
                    let at = (frame * 2048 + i * 2 + ch) * 4;
                    let expected = f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()) as f64;
                    let foreign =
                        f32::from_le_bytes(external[at..at + 4].try_into().unwrap()) as f64;
                    external_peaks[ch] = external_peaks[ch].max((pcm[ch][i] - foreign).abs());
                    if name != "independent" || frame < 5 || ch == 0 {
                        assert!(
                            (pcm[ch][i] - foreign).abs() < 1e-7,
                            "external {name} frame={frame} ch={ch} sample={i}"
                        );
                    }
                    assert!(
                        (pcm[ch][i] - expected).abs() < 1e-7,
                        "{name} frame={frame} ch={ch} sample={i}: {} vs {expected}",
                        pcm[ch][i]
                    );
                }
            }
            for len in 1..raw.len() - 2 {
                let mut bad = BitReader::new(&raw[..len]);
                bad.skip(7).unwrap();
                assert!(ChannelPair::read_ltp(&mut bad, &config).is_err());
                assert_eq!(bad.position(), 7);
            }
        }
        if name == "independent" {
            // Reproduce the saved external reference's right-channel flag disagreement.
            assert!(external_peaks[1] > 4e-4);
            assert!(external_peaks[0] < 1e-7);
        }
    }
}

#[test]
fn ltp_pair_reserved_mask_and_profile_refuse_without_consuming_bits() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
    let blob = bytes("aac-ltp-pair-packets.bin");
    let row = &m["cases"][0]["frames"][0];
    let at = row["offset"].as_u64().unwrap() as usize;
    let mut raw = blob[at..at + row["bytes"].as_u64().unwrap() as usize].to_vec();
    // First packet has a common long ICS without prediction: MS mode starts at bit 19.
    for bit in [19, 20] {
        raw[bit / 8] |= 1 << (7 - bit % 8);
    }
    let mut config = AacConfig {
        object_type: 4,
        sample_rate: 24000,
        channels: 2,
        channel_configuration: 2,
        frame_samples: 1024,
        core_coder_delay: None,
    };
    let mut b = BitReader::new(&raw);
    b.skip(7).unwrap();
    assert!(
        ChannelPair::read_ltp(&mut b, &config)
            .err()
            .unwrap()
            .to_string()
            .contains("reserved AAC mid/side mask mode")
    );
    assert_eq!(b.position(), 7);
    config.object_type = 2;
    let mut b = BitReader::new(&raw);
    b.skip(7).unwrap();
    assert!(ChannelPair::read_ltp(&mut b, &config).is_err());
    assert_eq!(b.position(), 7);
}
