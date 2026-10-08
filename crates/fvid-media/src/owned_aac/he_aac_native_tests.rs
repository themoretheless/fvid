use super::*;
use serde_json::Value;
const PACKETS: &[u8] =
    include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.bin");
const PCM: &[u8] =
    include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
fn cases() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.json"
    ))
    .unwrap()
}
fn asc(case: &Value) -> Vec<u8> {
    case["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packet(frame: &Value) -> &[u8] {
    let at = frame["offset"].as_u64().unwrap() as usize;
    &PACKETS[at..at + frame["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn complete_authored_lc_plus_sbr_blocks_match_independent_pcm_and_clock() {
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap() {
        let config = asc(case);
        let mut decoder = NativeAacDecoder::new(&config).unwrap();
        let rate = if case["bands"] == 64 { 48000 } else { 24000 };
        assert_eq!((decoder.sample_rate(), decoder.channels()), (rate, 1));
        let mut output = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let saved = decoder.checkpoint();
            let decoded = decoder.decode(packet(frame)).unwrap();
            decoder.reset();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoded, decoder.decode(packet(frame)).unwrap());
            output.extend(decoded);
        }
        assert_eq!(output.len(), case["samples"].as_u64().unwrap() as usize);
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        for (&value, bytes) in output
            .iter()
            .zip(PCM[offset..offset + 8 * output.len()].chunks_exact(8))
        {
            let expected = f64::from_le_bytes(bytes.try_into().unwrap()) as f32;
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
}
#[test]
fn crc_failure_preserves_aac_and_sbr_histories_and_rejects_mismatched_checkpoints() {
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&asc(case)).unwrap();
        let frames = case["frames"].as_array().unwrap();
        decoder.decode(packet(&frames[0])).unwrap();
        let before = decoder.checkpoint();
        let raw = packet(&frames[1]);
        let mut broken = raw.to_vec();
        // The original silent SCE is 29 bits, FIL/type/count lead to bit40:
        // flip a stored CRC bit without changing core or SBR syntax.
        broken[5] ^= 0x80;
        assert!(
            decoder
                .decode(&broken)
                .unwrap_err()
                .to_string()
                .contains("CRC")
        );
        let expected = decoder.decode(raw).unwrap();
        decoder.restore(&before).unwrap();
        assert_eq!(decoder.decode(raw).unwrap(), expected);
        let mut lc = NativeAacDecoder::new(&[0x13, 0x08]).unwrap();
        assert!(lc.restore(&before).is_err());
    }
}
#[test]
fn extended_asc_keeps_core_rate_ps_and_unspecified_flags_distinct() {
    fn packed(text: &str) -> Vec<u8> {
        let mut bits = text.to_owned();
        while bits.len() % 8 != 0 {
            bits.push('0');
        }
        bits.as_bytes()
            .chunks_exact(8)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 2).unwrap())
            .collect()
    }
    // Explicit AOT5, indexed24k core /48k extension, LC, 960 frame.
    let explicit = packed("00101 0110 0001 0011 00010 100".replace(' ', "").as_str());
    let parsed = AudioSpecificConfig::parse(&explicit).unwrap();
    assert_eq!(
        (
            parsed.core.sample_rate,
            parsed.output_sample_rate(),
            parsed.core.frame_samples
        ),
        (24000, 48000, 960)
    );
    assert_eq!((parsed.sbr_present, parsed.ps_present), (Some(true), None));
    assert!(AacConfig::parse(&explicit).is_err()); // legacy metadata remains guarded
    let core = AudioSpecificConfig::parse(&[0x13, 0x08]).unwrap();
    assert_eq!((core.sbr_present, core.ps_present), (None, None));
    // Core LC24k mono + sync SBR absent: preserve an explicit false flag.
    let absent = packed(
        "00010 0110 0001 000 01010110111 00101 0"
            .replace(' ', "")
            .as_str(),
    );
    let parsed = AudioSpecificConfig::parse(&absent).unwrap();
    assert_eq!(parsed.sbr_present, Some(false));
    assert_eq!(parsed.output_sample_rate(), 24000);
    assert!(AacConfig::parse(&absent).is_ok());
    let ps = packed("11101 0110 0001 0011 00010 000".replace(' ', "").as_str());
    let parsed = AudioSpecificConfig::parse(&ps).unwrap();
    assert_eq!(
        (parsed.ps_present, parsed.output_channels()),
        (Some(true), 2)
    );
    assert!(
        NativeAacDecoder::new(&ps)
            .err()
            .unwrap()
            .to_string()
            .contains("parametric stereo")
    );
    let sync_ps = packed(
        "00010 0110 0001 000 01010110111 00101 1 0011 10101001000 1"
            .replace(' ', "")
            .as_str(),
    );
    let parsed = AudioSpecificConfig::parse(&sync_ps).unwrap();
    assert_eq!(parsed.ps_present, Some(true));
    // Independent 24-bit core/extension frequency escapes.
    let text = format!(
        "00101 1111 {:024b} 0001 1111 {:024b} 00010 000",
        24000, 48000
    )
    .replace(' ', "");
    let escaped = packed(&text);
    let parsed = AudioSpecificConfig::parse(&escaped).unwrap();
    assert_eq!(
        (parsed.core.sample_rate, parsed.output_sample_rate()),
        (24000, 48000)
    );
    for end in 0..escaped.len() {
        assert!(AudioSpecificConfig::parse(&escaped[..end]).is_err());
    }
    let zero = packed(&format!("00101 1111 {:024b} 0001 0011 00010 000", 0).replace(' ', ""));
    assert!(AudioSpecificConfig::parse(&zero).is_err());
    let reserved = packed("00101 1101 0001 0011 00010 000".replace(' ', "").as_str());
    assert!(AudioSpecificConfig::parse(&reserved).is_err());
}

#[test]
fn complete_stereo_cpe_sbr_matches_both_independent_mono_oracles_and_crc_rollback() {
    let manifest: Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/playback-errors/he-aac-sbr-stereo.json"
    ))
    .unwrap();
    let data = include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-stereo.bin");
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&asc(case)).unwrap();
        assert_eq!(decoder.channels(), 2);
        let mut output = Vec::new();
        for (n, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let start = frame["offset"].as_u64().unwrap() as usize;
            let raw = &data[start..start + frame["bytes"].as_u64().unwrap() as usize];
            let saved = decoder.checkpoint();
            if n == 1 {
                let bit = frame["crc_bit"].as_u64().unwrap() as usize;
                let mut broken = raw.to_vec();
                broken[bit / 8] ^= 1 << (7 - bit % 8);
                assert!(
                    decoder
                        .decode(&broken)
                        .unwrap_err()
                        .to_string()
                        .contains("CRC")
                );
            }
            let decoded = decoder
                .decode(raw)
                .unwrap_or_else(|e| panic!("frame {n}, case {case}: {e}"));
            decoder.reset();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoded, decoder.decode(raw).unwrap());
            output.extend(decoded);
        }
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        assert_eq!(output.len(), 2 * case["samples"].as_u64().unwrap() as usize);
        for (pair, b) in output.chunks_exact(2).zip(PCM[offset..].chunks_exact(8)) {
            let expected = (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_bits();
            assert_eq!(pair[0].to_bits(), expected, "left {case}");
            assert_eq!(pair[1].to_bits(), expected, "right {case}");
        }
    }
}
