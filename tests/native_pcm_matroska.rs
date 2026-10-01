use fvid::{
    container::{
        matroska_write::{Encoding, PacketWriter, TrackSpec},
        webm,
    },
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
    let path = std::env::temp_dir().join(format!("fvid-pcm-mka-{name}-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    Dir(path)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn export(source: &Path, output: &Path) -> fvid::native_media::AudioDecodeStats {
    native_export::export_audio_pcm_selected(
        source, output, None, 1.0, None, None, None, None, None,
    )
    .unwrap()
}
#[test]
fn pcm_mux_round_trips_aac_edits_alac_and_multichannel_samples_exactly() {
    let d = dir("roundtrip");
    for (i, name) in [
        "audio/aac-stereo.aac",
        "audio/aac-native-edit.m4a",
        "alac/stereo-24.m4a",
        "alac/stereo-24.mka",
        "audio/aac-51-active.aac",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let reference = d.0.join(format!("raw-{i}.f32le"));
        let muxed = d.0.join(format!("pcm-{i}.mka"));
        let decoded = d.0.join(format!("decoded-{i}.f32le"));
        let original = export(&source, &reference);
        let written = export(&source, &muxed);
        let read = export(&muxed, &decoded);
        assert_eq!(original.sample_frames, written.sample_frames);
        assert_eq!(read.sample_frames, written.sample_frames);
        let raw = std::fs::read(reference).unwrap();
        let actual = std::fs::read(decoded).unwrap();
        assert!(raw == actual, "{name} samples differ");
        let bytes = std::fs::read(&muxed).unwrap();
        let mut reader = webm::WebmReader::open(Cursor::new(bytes), Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].codec, "A_PCM/FLOAT/IEEE");
        assert_eq!(reader.tracks[0].bit_depth, 32);
        assert_eq!(reader.tracks[0].channels, u64::from(written.channels));
        let mut frames = 0u128;
        for packet in &reader.packets {
            assert_eq!(
                packet.pts_ns as u128,
                frames * 1_000_000_000 / u128::from(written.sample_rate)
            );
            frames += (packet.size / (usize::from(written.channels) * 4)) as u128;
            assert!(packet.size <= 1024 * usize::from(written.channels) * 4);
            assert_eq!(
                packet.pts_ns as u128 + u128::from(packet.duration_ns.unwrap()),
                frames * 1_000_000_000 / u128::from(written.sample_rate)
            );
        }
        assert_eq!(frames, u128::from(written.sample_frames));
        if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let result = std::process::Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&muxed)
                .args(["-f", "f32le", "-"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(result.stdout == raw, "independent PCM differs: {name}");
        }
    }
}
#[test]
fn invalid_pcm_packets_poison_writer_and_invalid_headers_emit_nothing() {
    for (rate, channels) in [(0, 2), (48000, 0), (48000, 65)] {
        let mut out = Cursor::new(Vec::new());
        let specs = [TrackSpec {
            encoding: Encoding::PcmFloat32 {
                sample_rate: rate,
                channels,
            },
            name: "",
            language: "und",
        }];
        assert!(PacketWriter::new(&mut out, &specs).is_err());
        assert!(out.into_inner().is_empty());
    }
    for (bytes, duration) in [
        (vec![0; 7], 20_833),
        (f32::NAN.to_le_bytes().repeat(2), 20_833),
        (vec![0; 8], 50_000),
    ] {
        let mut out = Cursor::new(Vec::new());
        let specs = [TrackSpec {
            encoding: Encoding::PcmFloat32 {
                sample_rate: 48000,
                channels: 2,
            },
            name: "",
            language: "und",
        }];
        let mut writer = PacketWriter::new(&mut out, &specs).unwrap();
        assert!(writer.write_packet(0, 0, duration, true, &bytes).is_err());
        assert!(writer.finish().is_err());
    }
}
#[test]
fn pcm_publication_progress_cancellation_and_no_overwrite() {
    let d = dir("publication");
    let source = fixture("audio/aac-stereo.aac");
    let output = d.0.join("pcm.mka");
    let events = Arc::new(Mutex::new(Vec::new()));
    let saved = events.clone();
    let published = output.clone();
    let hook = ProgressHook::new(move |event| {
        if event.done {
            assert!(published.exists());
        }
        saved.lock().unwrap().push(event);
    });
    native_export::export_audio_pcm_selected(
        &source,
        &output,
        None,
        1.0,
        None,
        None,
        None,
        None,
        Some(&hook),
    )
    .unwrap();
    assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
    let original = std::fs::read(&output).unwrap();
    assert!(
        native_export::export_audio_pcm_selected(
            &source, &output, None, 1.0, None, None, None, None, None
        )
        .is_err()
    );
    assert!(std::fs::read(&output).unwrap() == original);
    let cancel = CancelFlag::new();
    let captured = cancel.clone();
    let hook = ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 0 {
            captured.cancel();
        }
    });
    let aborted = d.0.join("aborted.mka");
    assert!(
        native_export::export_audio_pcm_selected(
            &source,
            &aborted,
            None,
            1.0,
            None,
            None,
            None,
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!aborted.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
#[test]
fn headless_cli_exports_owned_matroska_pcm() {
    let d = dir("cli");
    let output = d.0.join("pcm.mka");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(fixture("audio/aac-native-edit.m4a"))
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert!(output.exists());
}

#[test]
fn transformed_pcm_preserves_resampled_interval_exactly() {
    let d = dir("transformed");
    let source = fixture("audio/aac-stereo.aac");
    let raw = d.0.join("transformed.f32le");
    let muxed = d.0.join("transformed.mkv");
    let decoded = d.0.join("decoded.f32le");
    let interval = Some((
        std::time::Duration::from_millis(10),
        std::time::Duration::from_millis(70),
    ));
    let a = native_export::export_audio_pcm_selected(
        &source,
        &raw,
        interval,
        0.25,
        Some(1),
        Some(44100),
        Some(0),
        None,
        None,
    )
    .unwrap();
    let b = native_export::export_audio_pcm_selected(
        &source,
        &muxed,
        interval,
        0.25,
        Some(1),
        Some(44100),
        Some(0),
        None,
        None,
    )
    .unwrap();
    let c = export(&muxed, &decoded);
    assert_eq!(
        (a.sample_frames, a.channels, a.sample_rate),
        (b.sample_frames, b.channels, b.sample_rate)
    );
    assert_eq!(
        (b.sample_frames, b.channels, b.sample_rate),
        (c.sample_frames, c.channels, c.sample_rate)
    );
    assert_eq!(c.channels, 1);
    assert_eq!(c.sample_rate, 44100);
    assert_eq!(std::fs::read(raw).unwrap(), std::fs::read(decoded).unwrap());
}

#[test]
fn trim_pcm_matroska_matches_selected_source_samples() {
    let d = dir("trim");
    let source = fixture("audio/aac-native-edit.m4a");
    let raw = d.0.join("trim.f32le");
    let muxed = d.0.join("trim.mka");
    let decoded = d.0.join("decoded.f32le");
    let a =
        native_export::trim_audio_pcm(&source, &raw, 10000, 70000, Some(0), None, None).unwrap();
    let b =
        native_export::trim_audio_pcm(&source, &muxed, 10000, 70000, Some(0), None, None).unwrap();
    let c = export(&muxed, &decoded);
    assert_eq!(a.sample_frames, b.sample_frames);
    assert_eq!(b.sample_frames, c.sample_frames);
    assert_eq!(std::fs::read(raw).unwrap(), std::fs::read(decoded).unwrap());
    #[cfg(feature = "media")]
    {
        let public = d.0.join("public.mkv");
        let options = fvid::media::CopyOptions::default();
        let stats = fvid::media::trim(&source, &public, 10000, 70000, &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(
            std::fs::read(public).unwrap(),
            std::fs::read(&muxed).unwrap()
        );
    }
    let cli = d.0.join("cli.mka");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "trim"])
        .arg(&source)
        .arg(&cli)
        .args(["--from", "0.01", "--to", "0.07"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(cli).unwrap(), std::fs::read(&muxed).unwrap());
    let invalid = d.0.join("invalid.mka");
    assert!(
        native_export::trim_audio_pcm(&source, &invalid, 70000, 10000, Some(0), None, None)
            .is_err()
    );
    assert!(!invalid.exists());
}
