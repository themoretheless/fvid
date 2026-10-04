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

#[test]
fn mp4_relocation_counts_media_bytes_and_cancels_without_publication() {
    use fvid::native_export::remux_mp4_controlled;
    let fixture = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let mdat = fixture.windows(4).position(|b| b == b"mdat").unwrap();
    let media_size = u32::from_be_bytes(fixture[mdat - 4..mdat].try_into().unwrap()) as u64 - 8;
    let extra = 3u32 << 20;
    let total = media_size + u64::from(extra);
    let mut source_bytes = fixture.to_vec();
    source_bytes.extend_from_slice(&(extra + 8).to_be_bytes());
    source_bytes.extend_from_slice(b"mdat");
    source_bytes.resize(source_bytes.len() + extra as usize, 0);
    for (name, stop) in [
        ("mp4-success", None),
        ("mp4-start", Some(0)),
        ("mp4-mid", Some(1 << 20)),
        ("mp4-end", Some(total)),
    ] {
        let dir = directory(name);
        let source = dir.0.join("input.mp4");
        let output = dir.0.join("output.mp4");
        std::fs::write(&source, &source_bytes).unwrap();
        let flag = CancelFlag::new();
        let shared = flag.clone();
        let events = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
        let captured = events.clone();
        let destination = output.clone();
        let hook = ProgressHook::new(move |event| {
            assert_eq!(event.packets, 0);
            assert_eq!(destination.exists(), event.done);
            captured.lock().unwrap().push(event);
            if stop.is_some_and(|limit| event.payload_bytes >= limit) {
                shared.cancel();
            }
        });
        let result = remux_mp4_controlled(&source, &output, Some(&flag), Some(&hook));
        let events = events.lock().unwrap();
        assert_eq!(events[0].payload_bytes, 0);
        for pair in events.windows(2) {
            assert!(pair[0].payload_bytes <= pair[1].payload_bytes);
        }
        if stop.is_some() {
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            assert!(!output.exists());
            assert!(events.iter().all(|event| !event.done));
            assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
        } else {
            result.unwrap();
            assert_eq!(
                events.last().unwrap(),
                &ProgressEvent {
                    packets: 0,
                    payload_bytes: total,
                    done: true
                }
            );
            assert_eq!(events.iter().filter(|event| event.done).count(), 1);
            assert!(
                events
                    .iter()
                    .any(|event| event.payload_bytes >= 1 << 20 && event.payload_bytes < total)
            );
            let mut expected = Vec::new();
            fvid::container::mp4_relocate::fast_start(
                &mut std::io::Cursor::new(&source_bytes),
                &mut expected,
            )
            .unwrap();
            assert_eq!(std::fs::read(output).unwrap(), expected);
        }
    }
}

#[test]
fn mp4_cli_progress_stays_on_native_backend() {
    let dir = directory("mp4-cli");
    let source = dir.0.join("input.mp4");
    let output = dir.0.join("output.mp4");
    std::fs::write(&source, include_bytes!("fixtures/hevc/main10-ipb.mp4")).unwrap();
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
    assert!(
        String::from_utf8(run.stdout)
            .unwrap()
            .contains("mp4-faststart")
    );
    assert!(
        String::from_utf8(run.stderr)
            .unwrap()
            .lines()
            .last()
            .unwrap()
            .ends_with("\"done\":true}")
    );
    assert!(output.exists());
}

#[test]
fn aac_pcm_progress_covers_all_containers_and_done_follows_publication() {
    use fvid::native_export::{export_aac_pcm_controlled, export_aac_pcm_resampled};
    for (name, file) in [
        ("adts", "aac-mono-44k.aac"),
        ("mp4", "aac-native-edit.m4a"),
        ("mka", "aac-stereo.mka"),
    ] {
        let dir = directory(&format!("pcm-{name}"));
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(file);
        let output = dir.0.join("controlled.wav");
        let baseline = dir.0.join("baseline.wav");
        let events = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
        let captured = events.clone();
        let destination = output.clone();
        let hook = ProgressHook::new(move |event| {
            assert_eq!(destination.exists(), event.done);
            captured.lock().unwrap().push(event);
        });
        let stats = export_aac_pcm_controlled(
            &source,
            &output,
            None,
            0.5,
            Some(2),
            Some(16000),
            None,
            Some(&hook),
        )
        .unwrap();
        let expected =
            export_aac_pcm_resampled(&source, &baseline, None, 0.5, Some(2), Some(16000)).unwrap();
        assert_eq!(stats, expected);
        assert_eq!(
            std::fs::read(output).unwrap(),
            std::fs::read(baseline).unwrap()
        );
        let events = events.lock().unwrap();
        assert_eq!(
            events[0],
            ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false
            }
        );
        let final_event = events.last().unwrap();
        assert_eq!(final_event.packets, stats.decoded_frames);
        assert!(final_event.payload_bytes > 0);
        assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    }
}

#[test]
fn aac_pcm_cancellation_cleans_files_in_each_container_path() {
    use fvid::native_export::export_aac_pcm_controlled;
    for (name, file) in [
        ("adts", "aac-mono-44k.aac"),
        ("mp4", "aac-native-edit.m4a"),
        ("mka", "aac-stereo.mka"),
    ] {
        for before in [true, false] {
            let dir = directory(&format!("pcm-cancel-{name}-{before}"));
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .join(file);
            let flag = CancelFlag::new();
            if before {
                flag.cancel();
            }
            let shared = flag.clone();
            let hook = ProgressHook::new(move |event| {
                assert!(!event.done);
                if event.packets > 0 {
                    shared.cancel();
                }
            });
            let error = export_aac_pcm_controlled(
                &source,
                &dir.0.join("out.wav"),
                None,
                1.0,
                None,
                None,
                Some(&flag),
                Some(&hook),
            )
            .unwrap_err();
            assert!(error.to_string().contains("cancelled"));
            assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
        }
    }
    let dir = directory("pcm-cancel-mid");
    let source = dir.0.join("long.aac");
    std::fs::write(
        &source,
        include_bytes!("fixtures/audio/aac-mono-44k.aac").repeat(40),
    )
    .unwrap();
    let flag = CancelFlag::new();
    let shared = flag.clone();
    let count = Arc::new(Mutex::new(0));
    let captured = count.clone();
    let hook = ProgressHook::new(move |event| {
        assert!(!event.done);
        *captured.lock().unwrap() = event.packets;
        if event.packets >= 256 {
            shared.cancel();
        }
    });
    assert!(
        export_aac_pcm_controlled(
            &source,
            &dir.0.join("out.wav"),
            None,
            1.0,
            None,
            None,
            Some(&flag),
            Some(&hook)
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled")
    );
    assert_eq!(*count.lock().unwrap(), 256);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn aac_pcm_cli_progress_with_transforms_remains_native_and_quiet() {
    for (name, file) in [
        ("adts", "aac-mono-44k.aac"),
        ("mp4", "aac-native-edit.m4a"),
        ("mka", "aac-stereo.mka"),
    ] {
        let dir = directory(&format!("pcm-cli-{name}"));
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(file);
        let output = dir.0.join("out.wav");
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode-audio"])
            .arg(&source)
            .arg(&output)
            .args([
                "--progress",
                "--quiet",
                "--from",
                "0.001",
                "--to",
                "0.003",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--volume",
                "0.5",
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(run.stdout.is_empty());
        let events = String::from_utf8(run.stderr).unwrap();
        assert!(events.lines().last().unwrap().ends_with("\"done\":true}"));
        let wav = std::fs::read(output).unwrap();
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16000);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
    }
}
