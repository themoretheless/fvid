use fvid_codecs::flac::{Config, decode};
fn directory() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/flac-native")
}
#[test]
fn authored_flac_predictors_residuals_and_stereo_reconstruct_exact_pcm() {
    let mut count = 0;
    for entry in std::fs::read_dir(directory()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "case") {
            continue;
        }
        let case = std::fs::read(&path).unwrap();
        let config = Config {
            sample_rate: u32::from_be_bytes(case[..4].try_into().unwrap()),
            channels: u16::from(case[4]),
            bits: case[5],
        };
        let frame =
            decode(&case[10..], config).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let expected = std::fs::read(path.with_extension("s32le")).unwrap();
        assert_eq!(
            frame.frames,
            u32::from_be_bytes(case[6..10].try_into().unwrap()) as usize
        );
        let actual: Vec<u8> = frame.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert!(actual == expected, "{}: PCM mismatch", path.display());
        count += 1;
    }
    assert_eq!(count, 1460);
}
#[test]
fn truncated_or_corrupt_flac_never_returns_pcm() {
    let case = std::fs::read(directory().join("16-10-lpc8-1.case")).unwrap();
    let packet = &case[10..];
    let config = Config {
        sample_rate: 8000,
        channels: 2,
        bits: 16,
    };
    for end in 0..packet.len() {
        assert!(decode(&packet[..end], config).is_err(), "prefix {end}");
    }
    for at in 0..packet.len() {
        let mut bytes = packet.to_vec();
        bytes[at] ^= 1;
        assert!(decode(&bytes, config).is_err(), "bit flip at {at}");
    }
}

#[test]
fn valid_crc_does_not_admit_invalid_flac_syntax() {
    let config = Config {
        sample_rate: 8000,
        channels: 1,
        bits: 32,
    };
    for (name, expected) in [
        ("subframe-padding", "subframe padding"),
        ("subframe-type", "subframe type"),
        ("wasted-width", "wasted bits exhaust"),
        ("lpc-precision", "LPC precision"),
        ("lpc-shift", "LPC shift"),
        ("residual-method", "residual method"),
        ("partition-geometry", "partition geometry"),
        ("footer-padding", "footer padding"),
        ("trailing-payload", "trailing payload"),
    ] {
        let packet = std::fs::read(directory().join(format!("{name}.invalid"))).unwrap();
        let error = match decode(&packet, config) {
            Ok(_) => panic!("accepted {name}"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(expected), "{name}: {error}");
        assert!(!error.contains("CRC"), "unrelated CRC refusal for {name}");
    }
}
