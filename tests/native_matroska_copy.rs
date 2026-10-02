use fvid::{
    container::{matroska_copy, webm},
    media_control::{CancelFlag, ProgressHook},
    native_export,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-identity-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn exact_bytes_and_packet_counts_for_audio_and_video() {
    for name in [
        "audio/opus-flac.mkv",
        "audio/ac3-51.mka",
        "av1/random-access.webm",
    ] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let mut reader = webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let mut output = Vec::new();
        let stats =
            matroska_copy::copy(&mut Cursor::new(&bytes), &mut output, false, None, None).unwrap();
        let mut library_output = Vec::new();
        let library_stats = fvid_media::owned_matroska_copy::copy(
            &mut Cursor::new(&bytes), &mut library_output, false, None, None,
        ).unwrap();
        assert_eq!(library_output, bytes);
        assert_eq!(library_stats.packets, stats.packets);
        assert_eq!(library_stats.payload_bytes, stats.payload_bytes);
        assert!(!library_stats.done);
        assert_eq!(output, bytes);
        assert_eq!(stats.packets, reader.packets.len() as u64);
        assert_eq!(
            stats.payload_bytes,
            reader.packets.iter().map(|p| p.size as u64).sum::<u64>()
        );
        assert!(!stats.done);
    }
}
#[test]
fn atomic_publication_progress_and_no_overwrite() {
    let d = dir("publish");
    let dest = d.0.join("copy.mka");
    let source = fixture("audio/opus-flac.mkv");
    let expected = std::fs::read(&source).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let published = dest.clone();
    let hook = ProgressHook::new(move |event| {
        if event.done {
            assert_eq!(std::fs::read(&published).unwrap(), expected);
        }
        captured.lock().unwrap().push(event);
    });
    let stats = native_export::remux_matroska(&source, &dest, None, Some(&hook)).unwrap();
    assert!(stats.done);
    assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
    assert!(native_export::remux_matroska(&source, &dest, None, None).is_err());
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        std::fs::read(&source).unwrap()
    );
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
#[test]
fn invalid_input_video_to_audio_and_cancel_leave_no_output() {
    let d = dir("reject");
    let dest = d.0.join("copy.mka");
    assert!(
        native_export::remux_matroska(&fixture("av1/random-access.webm"), &dest, None, None)
            .is_err()
    );
    assert!(!dest.exists());
    let bad = d.0.join("bad.mkv");
    std::fs::write(&bad, [0x1a, 0x45, 0xdf, 0xa3, 0xff]).unwrap();
    assert!(native_export::remux_matroska(&bad, &dest, None, None).is_err());
    let cancel = CancelFlag::new();
    let c = cancel.clone();
    let hook = ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 0 {
            c.cancel();
        }
    });
    assert!(
        native_export::remux_matroska(
            &fixture("audio/opus-flac.mkv"),
            &dest,
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(cancel.is_cancelled());
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
#[test]
fn cli_and_plan_use_owned_path() {
    let d = dir("cli");
    let source = fixture("audio/opus-flac.mkv");
    let dest = d.0.join("copy.mkv");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&dest)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("matroska-preserve"));
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&dest).unwrap()
    );
    assert!(fvid::native_plan::remux(&source).unwrap().is_some());
}
#[cfg(feature = "media")]
#[test]
fn media_api_uses_owned_path() {
    let d = dir("api");
    let source = fixture("audio/opus-flac.mkv");
    let dest = d.0.join("copy.mkv");
    let stats = fvid::media::remux(&source, &dest, &Default::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(std::fs::read(source).unwrap(), std::fs::read(dest).unwrap());
}

#[test]
fn custom_packet_limit_is_checked_before_any_container_bytes_are_written() {
    let bytes = std::fs::read(fixture("av1/random-access.webm")).unwrap();
    let mut reader = webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let largest = reader.packets.iter().map(|p| p.size).max().unwrap();
    for limit in [0, largest - 1, largest, largest + 1] {
        let mut output = Vec::new();
        let result = matroska_copy::copy_with_packet_limit(
            &mut Cursor::new(&bytes), &mut output, false, None, None, limit);
        if limit < largest {
            assert!(result.is_err());
            assert!(output.is_empty());
        } else {
            assert!(result.is_ok());
            assert_eq!(output, bytes);
        }
    }
}

#[cfg(feature = "media")]
#[test]
fn public_remux_and_plan_own_custom_packet_budget_and_failure_publication() {
    let d = dir("packet-limit");
    let source = fixture("av1/random-access.webm");
    let bytes = std::fs::read(&source).unwrap();
    let mut reader = webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let largest = reader.packets.iter().map(|p| p.size).max().unwrap();
    for limit in [largest - 1, largest] {
        let dest = d.0.join(format!("copy-{limit}.mkv"));
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let published = dest.clone();
        let options = fvid::media::CopyOptions {
            max_packet_bytes: limit,
            progress: Some(ProgressHook::new(move |event| {
                if event.done { assert!(published.exists()); }
                captured.lock().unwrap().push(event);
            })),
            ..Default::default()
        };
        let plan = fvid::media::plan_remux(&source, &options);
        let result = fvid::media::remux(&source, &dest, &options);
        if limit < largest {
            assert!(plan.unwrap_err().contains("packet exceeds budget"));
            assert!(result.unwrap_err().contains("packet exceeds budget"));
            assert!(!dest.exists());
            assert!(events.lock().unwrap().iter().all(|event| !event.done));
            assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
        } else {
            assert!(plan.unwrap().steps.iter().any(|step| step.action == "budget"));
            assert_eq!(result.unwrap().backend, "fvid");
            assert_eq!(std::fs::read(dest).unwrap(), bytes);
            assert_eq!(events.lock().unwrap().iter().filter(|event| event.done).count(), 1);
        }
    }
}
