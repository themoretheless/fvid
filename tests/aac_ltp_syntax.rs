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
#[ignore = "AAC LTP history, prediction, synthesis and profile admission are not integrated"]
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
