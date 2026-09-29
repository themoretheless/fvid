use fvid::{
    media_control::{CancelFlag, ProgressEvent, ProgressHook},
    native_export::remux_adts_aac_controlled,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory(name: &str) -> Cleanup {
    let path = std::env::temp_dir().join(format!("fvid-control-{name}-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    Cleanup(path)
}

#[test]
fn progress_is_monotonic_and_done_means_destination_is_published() {
    let dir = directory("progress");
    let source = dir.0.join("input.aac");
    let output = dir.0.join("output.m4a");
    let data = include_bytes!("fixtures/audio/aac-mono-44k.aac").repeat(100);
    let input = fvid::container::adts::Aac::parse(&data, &Default::default()).unwrap();
    let bytes: u64 = (0..input.packets())
        .map(|i| input.packet(i).len() as u64)
        .sum();
    std::fs::write(&source, data).unwrap();
    let events = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
    let captured = events.clone();
    let destination = output.clone();
    let hook = ProgressHook::new(move |event| {
        assert_eq!(destination.exists(), event.done);
        captured.lock().unwrap().push(event);
    });
    let packets = remux_adts_aac_controlled(&source, &output, None, Some(&hook)).unwrap();
    assert_eq!(packets, input.packets() as u64);
    let events = events.lock().unwrap();
    assert_eq!(
        events[0],
        ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false
        }
    );
    assert_eq!(
        events.last().unwrap(),
        &ProgressEvent {
            packets,
            payload_bytes: bytes,
            done: true
        }
    );
    assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    assert!(events.iter().any(|e| e.packets == 256));
    for pair in events.windows(2) {
        assert!(pair[0].packets <= pair[1].packets);
        assert!(pair[0].payload_bytes <= pair[1].payload_bytes);
    }
}

#[test]
fn cancellation_before_during_and_after_muxing_never_publishes_output() {
    for (name, threshold, repeats) in [("before", 0, 1), ("during", 256, 100), ("finish", 7, 1)] {
        let dir = directory(name);
        let source = dir.0.join("input.aac");
        let output = dir.0.join("output.m4a");
        std::fs::write(
            &source,
            include_bytes!("fixtures/audio/aac-mono-44k.aac").repeat(repeats),
        )
        .unwrap();
        let cancel = CancelFlag::new();
        if threshold == 0 {
            cancel.cancel();
        }
        let shared = cancel.clone();
        let hook = ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets >= threshold {
                shared.cancel();
            }
        });
        let error =
            remux_adts_aac_controlled(&source, &output, Some(&cancel), Some(&hook)).unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error}");
        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}

#[cfg(feature = "media")]
#[test]
fn legacy_media_reexports_the_same_control_types() {
    let flag: fvid::media::CancelFlag = CancelFlag::new();
    let options = fvid::media::CopyOptions {
        cancel: Some(flag.clone()),
        ..Default::default()
    };
    flag.cancel();
    assert!(options.cancel.unwrap().is_cancelled());
    let _: fvid::media::ProgressHook = ProgressHook::new(|_: ProgressEvent| {});
}

#[test]
fn cli_progress_works_without_media_feature() {
    let dir = directory("cli");
    let source = dir.0.join("input.aac");
    let output = dir.0.join("output.m4a");
    std::fs::write(&source, include_bytes!("fixtures/audio/aac-mono-44k.aac")).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&output)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let events = String::from_utf8(run.stderr).unwrap();
    let lines: Vec<_> = events.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        "{\"packets\":0,\"payload_bytes\":0,\"done\":false}"
    );
    assert!(lines[1].starts_with("{\"packets\":7,"));
    assert!(lines[2].ends_with("\"done\":true}"));
    assert!(output.exists());
}
