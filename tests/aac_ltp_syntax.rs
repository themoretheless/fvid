use fvid_media::owned_aac::{
    aac_ltp_syntax::{LtpData, Usage},
    aac_synthesis::WindowSequence,
    bits::BitReader,
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
fn manifest() -> Value {
    serde_json::from_slice(&bytes("aac-ltp-syntax.json")).unwrap()
}
#[test]
fn authored_ltp_syntax_long_short_boundaries_and_transactional_truncation() {
    let blob = bytes("aac-ltp-syntax.bin");
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 240);
    for c in m["cases"].as_array().unwrap() {
        let at = c["offset"].as_u64().unwrap() as usize;
        let raw = &blob[at..at + c["bytes"].as_u64().unwrap() as usize];
        let seq = if c["sequence"] == 2 {
            WindowSequence::EightShort
        } else {
            WindowSequence::OnlyLong
        };
        let n = c["n"].as_u64().unwrap() as u16;
        let bands = c["bands"].as_u64().unwrap() as u8;
        let mut b = BitReader::new(raw);
        assert_eq!(b.read(3).unwrap(), 5);
        let data = LtpData::read(&mut b, seq, bands, n).unwrap();
        assert_eq!(b.position(), c["bits"].as_u64().unwrap() as usize);
        assert_eq!(u64::from(data.lag), c["lag"].as_u64().unwrap());
        assert_eq!(
            u64::from(data.coefficient_index),
            c["coefficient"].as_u64().unwrap()
        );
        match data.usage {
            Usage::Bands(used) => assert_eq!(
                used,
                c["used"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect::<Vec<_>>()
            ),
            Usage::Windows(windows) => {
                for (got, want) in windows.iter().zip(c["windows"].as_array().unwrap()) {
                    assert_eq!(got.is_some(), want["enabled"].as_bool().unwrap());
                    if let Some(got) = got {
                        assert_eq!(got.lag_offset, want["lag"].as_u64().map(|x| x as u8));
                    }
                }
            }
        }
        let mut bad = BitReader::new(&raw[..raw.len() - 1]);
        bad.read(3).unwrap();
        let before = bad.position();
        assert!(LtpData::read(&mut bad, seq, bands, n).is_err());
        assert_eq!(bad.position(), before);
    }
    let c = &m["invalid_lag"];
    let at = c["offset"].as_u64().unwrap() as usize;
    let raw = &blob[at..at + c["bytes"].as_u64().unwrap() as usize];
    let mut b = BitReader::new(raw);
    b.read(3).unwrap();
    assert!(
        LtpData::read(&mut b, WindowSequence::OnlyLong, 1, 960)
            .unwrap_err()
            .to_string()
            .contains("lag exceeds")
    );
    assert_eq!(b.position(), 3);
    for (n, bands, seq) in [
        (512, 2, WindowSequence::OnlyLong),
        (1024, 64, WindowSequence::OnlyLong),
        (1024, 16, WindowSequence::EightShort),
    ] {
        let mut b = BitReader::new(raw);
        assert!(LtpData::read(&mut b, seq, bands, n).is_err());
        assert_eq!(b.position(), 0);
    }
}
#[test]
fn ltp_profile_video_reproduces_configuration_refusal() {
    for c in manifest()["videos"].as_array().unwrap() {
        let mut pcm = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut pcm,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only AAC Main, LC and SSR core configurations are implemented"),
            "{error}"
        );
        assert!(pcm.is_empty());
    }
}
#[test]
#[ignore = "AAC LTP profile admission and remaining tool/layout qualification are pending"]
fn ltp_video_playback_acceptance_pending() {
    for c in manifest()["videos"].as_array().unwrap() {
        let mut pcm = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut pcm,
        )
        .unwrap();
        assert_eq!(pcm.len(), 12288 * 4);
        assert!(
            pcm.chunks_exact(4)
                .any(|x| f32::from_le_bytes(x.try_into().unwrap()).abs() > 1e-6)
        );
    }
}

#[test]
fn ltp_ics_preserves_independent_pair_predictors_and_short_grouping() {
    use fvid_media::owned_aac::{aac_ltp_syntax::LtpIcsInfo, aac_synthesis::WindowShape};
    let blob = bytes("aac-ltp-ics.bin");
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-ics.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 896);
    for c in m["cases"].as_array().unwrap() {
        let at = c["offset"].as_u64().unwrap() as usize;
        let raw = &blob[at..at + c["bytes"].as_u64().unwrap() as usize];
        let n = c["n"].as_u64().unwrap() as u16;
        let pair = c["pair"].as_bool().unwrap();
        let mut b = BitReader::new(raw);
        b.skip(3).unwrap();
        let parsed = LtpIcsInfo::read(&mut b, (63, 15), n, pair).unwrap();
        assert_eq!(b.position(), c["end"].as_u64().unwrap() as usize);
        assert_eq!(b.read(6).unwrap(), 43, "consumed following section bits");
        let seq = match c["sequence"].as_u64().unwrap() {
            0 => WindowSequence::OnlyLong,
            1 => WindowSequence::LongStart,
            2 => WindowSequence::EightShort,
            _ => WindowSequence::LongStop,
        };
        assert_eq!(parsed.info.sequence, seq);
        assert_eq!(
            parsed.info.shape,
            if c["shape"] == 0 {
                WindowShape::Sine
            } else {
                WindowShape::Kbd
            }
        );
        assert_eq!(parsed.info.max_sfb as u64, c["bands"].as_u64().unwrap());
        assert!(parsed.info.prediction.is_none());
        assert_eq!(
            parsed.info.group_lengths,
            c["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        );
        for (got, want) in parsed
            .channels
            .iter()
            .zip(c["predictors"].as_array().unwrap())
        {
            if want.is_null() {
                assert!(got.is_none());
                continue;
            }
            let got = got.as_ref().unwrap();
            assert_eq!(got.lag as u64, want["lag"].as_u64().unwrap());
            assert_eq!(
                got.coefficient_index as u64,
                want["coefficient"].as_u64().unwrap()
            );
            assert_eq!(
                got.usage,
                Usage::Bands(
                    want["used"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_bool().unwrap())
                        .collect()
                )
            );
        }
        // Every byte prefix that stops inside this ICS must roll back completely.
        for len in 1..raw.len() {
            if len * 8 >= c["end"].as_u64().unwrap() as usize {
                continue;
            }
            let mut truncated = BitReader::new(&raw[..len]);
            truncated.skip(3).unwrap();
            assert!(LtpIcsInfo::read(&mut truncated, (63, 15), n, pair).is_err());
            assert_eq!(truncated.position(), 3);
        }
        let mut limited = BitReader::new(raw);
        limited.skip(3).unwrap();
        if parsed.info.max_sfb > 0 {
            let bands = if seq == WindowSequence::EightShort {
                (63, parsed.info.max_sfb - 1)
            } else {
                (parsed.info.max_sfb - 1, 15)
            };
            assert!(LtpIcsInfo::read(&mut limited, bands, n, pair).is_err());
            assert_eq!(limited.position(), 3);
        }
    }
}
#[test]
fn malformed_ltp_ics_does_not_advance_cursor() {
    use fvid_media::owned_aac::aac_ltp_syntax::LtpIcsInfo;
    for (raw, bands, n) in [
        (&[0x80u8][..], (63, 15), 1024),
        (&[0u8; 16][..], (64, 15), 1024),
        (&[0u8; 16][..], (63, 16), 1024),
        (&[0u8; 16][..], (63, 15), 512),
    ] {
        let mut b = BitReader::new(raw);
        assert!(LtpIcsInfo::read(&mut b, bands, n, true).is_err());
        assert_eq!(b.position(), 0);
    }
}

#[test]
fn ltp_video_packets_parse_active_and_inactive_ics() {
    use fvid_media::owned_aac::aac_ltp_syntax::LtpIcsInfo;
    let m = manifest();
    let blob = bytes("aac-ltp-packets.bin");
    for video in m["videos"].as_array().unwrap() {
        for (i, row) in video["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut b = BitReader::new(raw);
            assert_eq!(b.read(7).unwrap(), 0);
            b.read(8).unwrap();
            let parsed = LtpIcsInfo::read(&mut b, (47, 15), 1024, false).unwrap();
            assert_eq!(parsed.info.max_sfb, 2);
            let active = video["name"] == "active" && i >= 3;
            assert_eq!(parsed.channels[0].is_some(), active);
            assert!(parsed.channels[1].is_none());
            if active {
                let p = parsed.channels[0].as_ref().unwrap();
                assert_eq!(p.lag, 1024);
                assert_eq!(p.coefficient_index, i as u8 % 8);
                assert_eq!(p.usage, Usage::Bands(vec![true, true]));
            }
            assert_eq!(
                b.read(4).unwrap(),
                1,
                "next section codebook must remain aligned"
            );
        }
    }
}
