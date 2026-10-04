use fvid::native_pcm::trim_wave;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir() -> Dir {
    let p = std::env::temp_dir().join(format!(
        "fvid-pcm-trim-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend(tag);
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    if body.len() % 2 != 0 {
        out.push(0);
    }
}
fn wave(bits: u16, channels: u16, float: bool, extensible: bool, frames: usize) -> Vec<u8> {
    let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
    let block = channels * (bits / 8);
    let rate = 8000u32;
    let mut fmt = Vec::new();
    let tag: u16 = if float { 3 } else { 1 };
    fmt.extend((if extensible { 0xfffeu16 } else { tag }).to_le_bytes());
    fmt.extend(channels.to_le_bytes());
    fmt.extend(rate.to_le_bytes());
    fmt.extend((rate * u32::from(block)).to_le_bytes());
    fmt.extend(block.to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    if extensible {
        fmt.extend(22u16.to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        fmt.extend(((1u32 << channels) - 1).to_le_bytes());
        fmt.extend(u32::from(tag).to_le_bytes());
        fmt.extend([0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    }
    chunk(&mut out, b"JUNK", b"abc");
    chunk(&mut out, b"fmt ", &fmt);
    if float {
        chunk(&mut out, b"fact", &(frames as u32).to_le_bytes());
    }
    let data: Vec<u8> = (0..frames * usize::from(block))
        .map(|i| (i * 37 + 11) as u8)
        .collect();
    chunk(&mut out, b"data", &data);
    let mut info = b"INFO".to_vec();
    chunk(&mut info, b"INAM", b"unchanged title\0");
    chunk(&mut out, b"LIST", &info);
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}
fn chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8,
        bytes.len()
    );
    let mut at = 12;
    let mut out = Vec::new();
    while at < bytes.len() {
        let n = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push((
            bytes[at..at + 4].try_into().unwrap(),
            bytes[at + 8..at + 8 + n].to_vec(),
        ));
        at += 8 + n + (n & 1);
    }
    assert_eq!(at, bytes.len());
    out
}
#[test]
fn all_packed_wave_widths_are_trimmed_without_touching_sample_bits_or_metadata() {
    let dir = dir();
    for (bits, channels, float, ext) in [
        (8, 1, false, false),
        (16, 2, false, false),
        (24, 3, false, true),
        (32, 2, false, true),
        (32, 2, true, false),
        (64, 1, true, true),
    ] {
        let original = wave(bits, channels, float, ext, 17);
        let source = dir.0.join("source.wav");
        let dest = dir.0.join(format!("{bits}-{float}.wav"));
        std::fs::write(&source, &original).unwrap();
        let stats = trim_wave(&source, &dest, 125, 1250, None, None).unwrap();
        assert_eq!(stats.sample_frames, 9);
        assert_eq!(
            stats.payload_bytes,
            9 * u64::from(channels) * u64::from(bits / 8)
        );
        assert_eq!(stats.fvid_payload_copies, 0);
        let result = std::fs::read(&dest).unwrap();
        let pairs = chunks(&result);
        let before = chunks(&original);
        assert_eq!(pairs.len(), before.len());
        for ((tag, data), (old_tag, old)) in pairs.iter().zip(before) {
            assert_eq!(*tag, old_tag);
            match tag {
                b"data" => {
                    let block = usize::from(channels) * usize::from(bits / 8);
                    assert_eq!(*data, old[block..10 * block]);
                }
                b"fact" => assert_eq!(*data, 9u32.to_le_bytes()),
                _ => assert_eq!(*data, old),
            }
        }
        let info = fvid::native_pcm::inspect(&mut std::io::Cursor::new(&result), None).unwrap();
        assert_eq!(info.sample_frames, 9);
        assert_eq!(info.bits_per_sample, bits);
        assert_eq!(info.float, float);
        assert!(trim_wave(&source, &dest, 0, 125, None, None).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), result);
    }
}
#[test]
fn strict_bounds_and_time_errors_publish_nothing() {
    let dir = dir();
    let source = dir.0.join("source.wav");
    let dest = dir.0.join("out.wav");
    let base = wave(16, 2, false, false, 17);
    for mutation in 0..5 {
        let mut bytes = base.clone();
        match mutation {
            0 => {
                bytes.pop();
            }
            1 => {
                let pos = bytes.windows(4).position(|w| w == b"fmt ").unwrap();
                bytes[pos + 8..pos + 10].copy_from_slice(&6u16.to_le_bytes());
            }
            2 => {
                let pos = bytes.windows(4).position(|w| w == b"data").unwrap();
                bytes[pos + 4..pos + 8].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            3 => {
                bytes[12..16].copy_from_slice(b"cue ");
            }
            _ => {
                bytes[0..4].copy_from_slice(b"RF64");
            }
        }
        std::fs::write(&source, bytes).unwrap();
        assert!(trim_wave(&source, &dest, 0, 125, None, None).is_err());
        assert!(!dest.exists());
    }
    std::fs::write(&source, base).unwrap();
    for (from, to) in [(-1, 125), (0, 0), (0, 1), (125, 126), (3000, 4000)] {
        assert!(trim_wave(&source, &dest, from, to, None, None).is_err());
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
#[test]
fn progress_and_cancellation_are_bounded_and_never_publish_partial_audio() {
    let dir = dir();
    let source = dir.0.join("source.wav");
    let dest = dir.0.join("out.wav");
    std::fs::write(&source, wave(16, 2, false, false, 100000)).unwrap();
    let cancel = fvid::media_control::CancelFlag::default();
    let flag = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    let progress = fvid::media_control::ProgressHook::new(move |e| {
        recorded.lock().unwrap().push(e);
        if e.payload_bytes > 0 {
            flag.cancel();
        }
    });
    assert!(
        trim_wave(
            &source,
            &dest,
            0,
            10_000_000,
            Some(&cancel),
            Some(&progress)
        )
        .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    let events = events.lock().unwrap();
    assert!(!events.iter().any(|e| e.done));
    assert!(events.last().unwrap().payload_bytes <= 65536);
    drop(events);
    let done = Arc::new(Mutex::new(Vec::new()));
    let recorded = done.clone();
    let published = dest.clone();
    let progress = fvid::media_control::ProgressHook::new(move |e| {
        if e.done {
            assert!(published.exists());
        }
        recorded.lock().unwrap().push(e);
    });
    let stats = trim_wave(&source, &dest, 0, 20_000_000, None, Some(&progress)).unwrap();
    assert_eq!(stats.sample_frames, 100000);
    let events = done.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    assert_eq!(events.last().unwrap().payload_bytes, 400000);
}
#[test]
fn headless_cli_and_media_api_select_native_wave_path() {
    let dir = dir();
    let source = dir.0.join("source.wav");
    let dest = dir.0.join("out.wav");
    std::fs::write(&source, wave(24, 2, false, true, 32)).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "trim-pcm"])
        .arg(&source)
        .arg(&dest)
        .args([
            "--from",
            "0.000125",
            "--to",
            "0.00125",
            "--streams",
            "0",
            "--progress",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["sample_frames"], 9);
    assert!(String::from_utf8_lossy(&result.stderr).contains("\"done\":true"));
    {
        let api = dir.0.join("api.wav");
        let stats = fvid::media::trim_pcm(&source, &api, 125, 1250, &Default::default()).unwrap();
        assert_eq!(stats.sample_frames, 9);
        assert_eq!(std::fs::read(api).unwrap(), std::fs::read(dest).unwrap());
    }
    let bad = dir.0.join("bad.wav");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "trim-pcm"])
        .arg(&source)
        .arg(&bad)
        .args(["--from", "0.0000001", "--to", "0.00125"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!bad.exists());
}

#[test]
fn plans_and_both_trim_commands_share_exact_sample_ranges() {
    let dir = dir();
    let source = dir.0.join("source.wav");
    let original = wave(24, 2, false, true, 32);
    std::fs::write(&source, &original).unwrap();
    for command in ["trim-pcm", "trim"] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", command])
            .arg(&source)
            .args(["--from", "0.000125", "--to", "0.01", "--streams", "0"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["command"], command);
        assert_eq!(json["streams"][0]["codec"], "pcm_s24le");
        assert!(
            json["steps"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("[1, 32)")
        );
        assert!(
            json["steps"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("31 retained frames, 186 PCM bytes")
        );
        let mut native = fvid::native_plan::trim_pcm(&source, 125, 10000, Some(0)).unwrap();
        native.command = command.to_owned();
        assert_eq!(json, serde_json::to_value(native).unwrap());
        {
            let plan = if command == "trim" {
                fvid::media::plan_trim(&source, 125, 10000, &Default::default())
            } else {
                fvid::media::plan_trim_pcm(&source, 125, 10000, &Default::default())
            }
            .unwrap();
            assert_eq!(json, serde_json::to_value(plan).unwrap());
        }
        assert_eq!(
            std::fs::read_dir(&dir.0).unwrap().count(),
            1,
            "planning must not create files"
        );
    }
    let mut outputs = Vec::new();
    for command in ["trim-pcm", "trim"] {
        let dest = dir.0.join(format!("{command}.wav"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", command])
            .arg(&source)
            .arg(&dest)
            .args(["--from", "0.000125", "--to", "0.01"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["payload_bytes"], 186);
        if command == "trim" {
            assert_eq!(json["backend"], "fvid");
            assert_eq!(json["segments"], 1);
        }
        outputs.push(std::fs::read(&dest).unwrap());
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    {
        let dest = dir.0.join("api-trim.wav");
        let stats = fvid::media::trim(&source, &dest, 125, 10000, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(&dest).unwrap(), outputs[0]);
    }
}

#[test]
fn plan_and_execution_reject_the_same_bad_intervals_and_structures() {
    let dir = dir();
    let source = dir.0.join("source.wav");
    let dest = dir.0.join("out.wav");
    for bytes in [wave(32, 2, true, true, 32), {
        let mut b = wave(16, 1, false, false, 32);
        b[12..16].copy_from_slice(b"cue ");
        b
    }] {
        std::fs::write(&source, bytes).unwrap();
        for (from, to) in [
            (0, 125),
            (0, 1),
            (125, 126),
            (-1, 125),
            (0, 0),
            (5000, 6000),
        ] {
            let plan = fvid::native_plan::trim_pcm(&source, from, to, None);
            let execution = trim_wave(&source, &dest, from, to, None, None);
            assert_eq!(plan.is_ok(), execution.is_ok(), "[{from},{to})");
            if execution.is_ok() {
                std::fs::remove_file(&dest).unwrap();
            }
        }
    }
    std::fs::write(&source, wave(32, 2, true, true, 32)).unwrap();
    let plan = fvid::native_plan::trim_pcm(&source, 0, 125, None).unwrap();
    assert_eq!(plan.streams[0].codec, "pcm_f32le");
    assert!(fvid::native_plan::trim_pcm(&source, 0, 125, Some(1)).is_err());
    for options in [
        vec!["--from", "0", "--to", "0.001", "--streams", "1"],
        vec!["--from", "0.0000001", "--to", "0.001"],
        vec!["--from", "0", "--to", "0.001", "--progress"],
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "trim-pcm"])
            .arg(&source)
            .args(options)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
