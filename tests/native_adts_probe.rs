use fvid::native_media::inspect_adts;
use std::io::Cursor;
const MONO: &[u8] = include_bytes!("fixtures/audio/aac-mono-44k.aac");
#[test]
fn sequential_probe_counts_samples_and_rejects_truncation_and_configuration_changes() {
    struct Short<R>(R);
    impl<R: std::io::Read> std::io::Read for Short<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(3); self.0.read(&mut buf[..n])
        }
    }
    let info = inspect_adts(Short(Cursor::new(MONO))).unwrap();
    assert_eq!((info.packets, info.sample_frames, info.sample_rate, info.channels), (7,7168,44100,1));
    assert_eq!(info.payload_bytes, MONO.len() as u64 - 7*7);
    assert!(inspect_adts(Cursor::new(&MONO[..MONO.len()-1])).is_err());
    let mut changed = MONO.to_vec();
    changed.extend_from_slice(include_bytes!("fixtures/audio/aac-stereo.aac"));
    assert!(inspect_adts(Cursor::new(changed)).is_err());
    let mut unsupported = MONO.to_vec(); unsupported[2] &= 0x3f; // AAC Main
    assert!(inspect_adts(Cursor::new(unsupported)).unwrap_err().to_string().contains("AAC-LC"));
}
#[cfg(feature="media")]
#[test]
fn public_probe_and_cli_use_exact_sample_clock() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    for info in [fvid::media::probe(&source).unwrap(), fvid::media::probe_as(&source, Some("aac")).unwrap()] {
        assert_eq!(info.format, "aac");
        assert_eq!(info.duration_us, Some(7168*1_000_000/44100));
        assert_eq!(info.streams[0].time_base, [1,44100]);
        assert_eq!(info.streams[0].duration, Some(7168));
        assert_eq!(info.streams[0].channels, 1);
        assert_eq!(info.streams[0].profile.as_deref(), Some("LC"));
        assert_eq!(info.streams[0].pixel_format, -1);
    }
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media","probe"]).arg(&source).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let json: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(json["streams"][0]["time_base"], serde_json::json!([1,44100]));
    let invalid = source.with_file_name("aac-native-edit.m4a");
    assert!(fvid::media::probe_as(&invalid, Some("aac")).is_err());
}
