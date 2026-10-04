use fvid::native_export::export_audio_pcm_selected as export;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
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
        "fvid-wave-audio-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn wave(bits: u16, channels: u16, float: bool, ext: Option<(u16, u32)>, data: &[u8]) -> Vec<u8> {
    let rate = 8000u32;
    let mut fmt = Vec::new();
    let tag = if float { 3u16 } else { 1 };
    fmt.extend(if ext.is_some() {
        0xfffeu16.to_le_bytes()
    } else {
        tag.to_le_bytes()
    });
    fmt.extend(channels.to_le_bytes());
    fmt.extend(rate.to_le_bytes());
    let align = channels * (bits / 8);
    fmt.extend((rate * u32::from(align)).to_le_bytes());
    fmt.extend(align.to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    if let Some((valid, mask)) = ext {
        fmt.extend(22u16.to_le_bytes());
        fmt.extend(valid.to_le_bytes());
        fmt.extend(mask.to_le_bytes());
        fmt.extend(u32::from(tag).to_le_bytes());
        fmt.extend([0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    }
    let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    out.extend((fmt.len() as u32).to_le_bytes());
    out.extend(fmt);
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    if data.len() % 2 != 0 {
        out.push(0);
    }
    let n = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&n.to_le_bytes());
    out
}
fn values(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn all_pcm_storage_types_convert_with_their_signedness_and_scale() {
    let dir = dir();
    let source = dir.0.join("input.wav");
    let cases: Vec<(u16, bool, Vec<u8>, Vec<f32>)> = vec![
        (
            8,
            false,
            vec![0, 64, 128, 192, 255],
            vec![-1., -0.5, 0., 0.5, 127. / 128.],
        ),
        (
            16,
            false,
            [-32768i16, -16384, 0, 16384, 32767]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect(),
            vec![-1., -0.5, 0., 0.5, 32767. / 32768.],
        ),
        (
            24,
            false,
            [-8388608i32, -4194304, 0, 4194304, 8388607]
                .into_iter()
                .flat_map(|n| n.to_le_bytes()[..3].to_vec())
                .collect(),
            vec![-1., -0.5, 0., 0.5, 8388607. / 8388608.],
        ),
        (
            32,
            false,
            [i32::MIN, -1073741824, 0, 1073741824, i32::MAX]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
            vec![-1., -0.5, 0., 0.5, 1.],
        ),
        (
            32,
            true,
            [-1.25f32, -0.5, 0., 1.25, 2.]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
            vec![-1.25, -0.5, 0., 1.25, 2.],
        ),
        (
            64,
            true,
            [-1.25f64, -0.5, 0., 1.25, 2.]
                .into_iter()
                .flat_map(f64::to_le_bytes)
                .collect(),
            vec![-1.25, -0.5, 0., 1.25, 2.],
        ),
    ];
    for (bits, float, data, expected) in cases {
        std::fs::write(&source, wave(bits, 1, float, None, &data)).unwrap();
        let dest = dir.0.join(format!("{bits}-{float}.f32le"));
        let stats = export(&source, &dest, None, 1., None, None, None, None, None).unwrap();
        assert_eq!(stats.sample_frames, 5);
        assert_eq!(stats.channels, 1);
        assert_eq!(values(&dest), expected);
        let plan = fvid::native_plan::decode_audio(&source, &Default::default()).unwrap();
        assert!(plan.steps[0].detail.contains("WAVE"));
        assert!(plan.streams[0].codec.starts_with("pcm_"));
    }
}
#[test]
fn fractional_interval_gain_rematrix_and_resampling_use_shared_pipeline() {
    let dir = dir();
    let source = dir.0.join("input.wav");
    let dest = dir.0.join("out.f32le");
    // Five stereo frames, with a constant mean of 0.25 in the retained interval.
    let data: [i16; 10] = [0, 0, 0, 16384, 16384, 0, 0, 0, 0, 0];
    std::fs::write(
        &source,
        wave(
            16,
            2,
            false,
            None,
            &data
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>(),
        ),
    )
    .unwrap();
    let stats = export(
        &source,
        &dest,
        Some((Duration::from_nanos(62500), Duration::from_nanos(312500))),
        2.,
        Some(1),
        Some(16000),
        Some(0),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        (stats.sample_frames, stats.channels, stats.sample_rate),
        (4, 1, 16000)
    );
    assert!(values(&dest).iter().all(|&v| (v - 0.5).abs() < 1e-6));
    let wav = dir.0.join("out.wav");
    export(&source, &wav, None, 1., None, None, None, None, None).unwrap();
    let round = dir.0.join("round.f32le");
    export(&wav, &round, None, 1., None, None, None, None, None).unwrap();
    let expected: Vec<f32> = data.iter().map(|&n| f32::from(n) / 32768.).collect();
    assert_eq!(values(&round), expected);
}
#[test]
fn padding_and_nonfinite_samples_fail_without_publication() {
    let dir = dir();
    let source = dir.0.join("input.wav");
    let dest = dir.0.join("out.f32le");
    for bytes in [
        wave(32, 1, true, None, &f32::NAN.to_le_bytes()),
        wave(64, 1, true, None, &1e300f64.to_le_bytes()),
        wave(24, 1, false, Some((20, 4)), &[1, 0, 0]),
    ] {
        std::fs::write(&source, bytes).unwrap();
        assert!(export(&source, &dest, None, 1., None, None, None, None, None).is_err());
        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    std::fs::write(&source, wave(24, 1, false, Some((20, 4)), &[0, 0, 64])).unwrap();
    export(&source, &dest, None, 1., None, None, None, None, None).unwrap();
    assert_eq!(values(&dest), vec![0.5]);
    let before = std::fs::read(&dest).unwrap();
    assert!(export(&source, &dest, None, 1., None, None, None, None, None).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), before);
}
#[test]
fn canonical_surround_layout_and_lfe_omission_are_explicit() {
    let dir = dir();
    let source = dir.0.join("input.wav");
    let dest = dir.0.join("out.f32le");
    let samples = [0f32, 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0.];
    std::fs::write(
        &source,
        wave(
            32,
            6,
            true,
            Some((32, 0x3f)),
            &samples
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        ),
    )
    .unwrap();
    export(&source, &dest, None, 1., Some(2), None, None, None, None).unwrap();
    assert_eq!(
        values(&dest),
        vec![
            0.,
            0.,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2
        ]
    );
}
#[test]
fn cli_plan_and_media_api_use_owned_pcm_conversion() {
    let dir = dir();
    let source = dir.0.join("input.wav");
    let dest = dir.0.join("out.f32le");
    std::fs::write(&source, wave(8, 1, false, None, &[0, 128, 255])).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&source)
        .arg(&dest)
        .args(["--volume", "0.5", "--streams", "0", "--progress"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(values(&dest), vec![-0.5, 0., 127. / 256.]);
    let stats: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert_eq!(stats["sample_frames"], 3);
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "decode-audio"])
        .arg(&source)
        .args(["--volume", "0.5", "--streams", "0"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(plan["streams"][0]["codec"], "pcm_u8");
    {
        let api = dir.0.join("api.f32le");
        let transform = fvid::media::AudioDecodeTransform {
            volume: Some(0.5),
            ..Default::default()
        };
        let stats =
            fvid::media::decode_audio_transformed(&source, &api, transform, &Default::default())
                .unwrap();
        assert_eq!(stats.sample_frames, 3);
        assert_eq!(values(&api), values(&dest));
        let api = fvid::media::plan_decode_audio(&source, &transform, &Default::default()).unwrap();
        assert_eq!(serde_json::to_value(api).unwrap(), plan);
    }
}

#[test]
fn invalid_intervals_selection_and_cancel_leave_no_export() {
    use fvid::native_plan::AudioDecodeTransform;
    use std::sync::{Arc, Mutex};
    let dir = dir();
    let source = dir.0.join("input.wav");
    let dest = dir.0.join("out.f32le");
    std::fs::write(&source, wave(16, 1, false, None, &[0; 16])).unwrap();
    let transform = AudioDecodeTransform {
        interval: Some((2000, 3000)),
        ..Default::default()
    };
    assert!(fvid::native_plan::decode_audio(&source, &transform).is_err());
    assert!(
        export(
            &source,
            &dest,
            Some((Duration::from_micros(2000), Duration::from_micros(3000))),
            1.,
            None,
            None,
            None,
            None,
            None
        )
        .is_err()
    );
    assert!(export(&source, &dest, None, 1., None, None, Some(1), None, None).is_err());
    let cancel = fvid::media_control::CancelFlag::new();
    let flag = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let record = events.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        record.lock().unwrap().push(e);
        if e.payload_bytes > 0 {
            flag.cancel();
        }
    });
    assert!(
        export(
            &source,
            &dest,
            None,
            1.,
            None,
            None,
            None,
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!events.lock().unwrap().iter().any(|e| e.done));
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "decode-audio"])
        .arg(&source)
        .args(["--from", "0.0001251", "--to", "0.00025"])
        .output()
        .unwrap();
    assert!(!run.status.success());
}

#[test]
fn wave_probe_uses_sample_clock_and_owned_dispatch() {
    let d = dir();
    let path = d.0.join("input.bin");
    for (bits, float, codec) in [
        (8, false, "pcm_u8"),
        (16, false, "pcm_s16le"),
        (24, false, "pcm_s24le"),
        (32, false, "pcm_s32le"),
        (32, true, "pcm_f32le"),
        (64, true, "pcm_f64le"),
    ] {
        std::fs::write(
            &path,
            wave(
                bits,
                2,
                float,
                None,
                &vec![0; 17 * 2 * usize::from(bits / 8)],
            ),
        )
        .unwrap();
        let info = fvid::native_probe::probe(&path).unwrap();
        assert_eq!(info.format, "wav");
        assert_eq!(info.duration_us, Some(2125));
        let stream = &info.streams[0];
        assert_eq!(stream.codec, codec);
        assert_eq!(stream.duration, Some(17));
        assert_eq!(stream.time_base, [1, 8000]);
        assert_eq!(stream.channels, 2);
        assert_eq!(stream.bit_rate, Some(8000 * 2 * i64::from(bits)));
        let expected = serde_json::to_value(info).unwrap();
        let cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "probe"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            cli.status.success(),
            "{}",
            String::from_utf8_lossy(&cli.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&cli.stdout).unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(fvid::native_probe::probe_as(&path, Some("wav")).unwrap())
                .unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(fvid::media::probe(&path).unwrap()).unwrap(),
            expected
        );
    }
    // Metadata inspection does not impose decoder channel layout restrictions.
    std::fs::write(&path, wave(16, 8, false, None, &[0; 32])).unwrap();
    assert_eq!(
        fvid::native_probe::probe(&path).unwrap().streams[0].channels,
        8
    );
    // Known WAVE corruption is never handed to the legacy adapter.
    let mut bytes = wave(16, 1, false, None, &[0; 4]);
    bytes.pop();
    std::fs::write(&path, bytes).unwrap();
    assert!(fvid::native_probe::try_probe_as(&path, None).is_err());
    assert!(fvid::media::probe(&path).is_err());
    std::fs::write(&path, b"RIFF\0\0\0\0AVI ").unwrap();
    assert!(
        fvid::native_probe::try_probe_as(&path, None)
            .unwrap()
            .is_none()
    );
    assert!(fvid::native_probe::probe_as(&path, Some("wav")).is_err());
}

#[test]
fn wave_concat_preserves_samples_and_matches_cli_plan_and_api() {
    let d = dir();
    for (bits, float) in [
        (8, false),
        (16, false),
        (24, false),
        (32, false),
        (32, true),
        (64, true),
    ] {
        let size = usize::from(bits / 8);
        let sources = vec![
            d.0.join(format!("{bits}-{float}-a.wav")),
            d.0.join(format!("{bits}-{float}-b.wav")),
        ];
        let a = vec![0x11; 3 * size];
        let b = vec![0x22; 4 * size];
        std::fs::write(&sources[0], wave(bits, 1, float, None, &a)).unwrap();
        std::fs::write(&sources[1], wave(bits, 1, float, None, &b)).unwrap();
        let expected = wave(bits, 1, float, None, &[a, b].concat());
        let dest = d.0.join(format!("{bits}-{float}-out.wav"));
        let stats = fvid::native_pcm::concat_wave(&sources, &dest, None, None).unwrap();
        assert_eq!(stats.sample_frames, 7);
        assert_eq!(stats.payload_bytes, 7 * size as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        assert!(fvid::native_pcm::concat_wave(&sources, &dest, None, None).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        let cli_dest = d.0.join(format!("{bits}-{float}-cli.wav"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "concat"])
            .arg(&cli_dest)
            .args(&sources)
            .args(["--streams", "0", "--progress"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(&cli_dest).unwrap(), expected);
        let plan = fvid::native_plan::concat_wave(&sources).unwrap();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "concat"])
            .args(&sources)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(),
            serde_json::to_value(&plan).unwrap()
        );
        {
            let api_dest = d.0.join(format!("{bits}-{float}-api.wav"));
            let stats = fvid::media::concat(&sources, &api_dest, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.segments, 2);
            assert_eq!(std::fs::read(api_dest).unwrap(), expected);
            assert_eq!(
                fvid::media::plan_concat(&sources, &Default::default()).unwrap(),
                plan
            );
        }
    }
}

#[test]
fn wave_concat_rejects_mismatch_and_cancels_without_publishing() {
    let d = dir();
    let sources = vec![d.0.join("a.wav"), d.0.join("b.wav")];
    let dest = d.0.join("result.wav");
    std::fs::write(&sources[0], wave(16, 2, false, None, &vec![1; 80000])).unwrap();
    for bytes in [
        wave(8, 2, false, None, &[0; 4]),
        wave(16, 1, false, None, &[0; 4]),
        wave(16, 2, false, Some((15, 3)), &[0; 4]),
        b"broken".to_vec(),
    ] {
        std::fs::write(&sources[1], bytes).unwrap();
        assert!(fvid::native_pcm::concat_wave(&sources, &dest, None, None).is_err());
        assert!(fvid::native_plan::concat_wave(&sources).is_err());
        assert!(!dest.exists());
    }
    std::fs::copy(&sources[0], &sources[1]).unwrap();
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.payload_bytes > 0 {
            stop.cancel();
        }
    });
    assert!(fvid::native_pcm::concat_wave(&sources, &dest, Some(&flag), Some(&hook)).is_err());
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 2);
    assert!(fvid::native_pcm::concat_wave(&sources[..1], &dest, None, None).is_err());
    // Repeated inputs are allowed and are read through separate file handles.
    let repeated = vec![sources[0].clone(), sources[0].clone()];
    assert_eq!(
        fvid::native_pcm::concat_wave(&repeated, &dest, None, None)
            .unwrap()
            .sample_frames,
        40000
    );
}

#[test]
fn wave_concat_rewrites_fact_and_keeps_first_metadata_after_data() {
    fn tagged(data: &[u8], tag: &[u8]) -> Vec<u8> {
        let mut bytes = wave(8, 1, false, None, data);
        bytes.extend(b"fact");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(b"LIST");
        bytes.extend((4 + tag.len() as u32).to_le_bytes());
        bytes.extend(b"INFO");
        bytes.extend(tag);
        if tag.len() % 2 != 0 {
            bytes.push(0);
        }
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }
    let d = dir();
    let sources = vec![d.0.join("a.wav"), d.0.join("b.wav")];
    let dest = d.0.join("out.wav");
    let tag = b"INAM\x04\0\0\0One\0";
    std::fs::write(&sources[0], tagged(&[1, 2, 3], tag)).unwrap();
    std::fs::write(&sources[1], tagged(&[4, 5, 6, 7], b"INAM\x04\0\0\0Two\0")).unwrap();
    fvid::native_pcm::concat_wave(&sources, &dest, None, None).unwrap();
    assert_eq!(
        std::fs::read(dest).unwrap(),
        tagged(&[1, 2, 3, 4, 5, 6, 7], tag)
    );
}

#[test]
fn wave_concat_rejects_riff_overflow_before_copying_payload() {
    use std::io::{Seek, SeekFrom, Write};
    let d = dir();
    let sources = vec![d.0.join("a.wav"), d.0.join("b.wav")];
    for path in &sources {
        let mut header = wave(8, 1, false, None, &[]);
        let payload = 0x80000000u32;
        header[4..8].copy_from_slice(&(payload + 36).to_le_bytes());
        header[40..44].copy_from_slice(&payload.to_le_bytes());
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&header).unwrap();
        f.seek(SeekFrom::Start(u64::from(payload) + 43)).unwrap();
        f.write_all(&[0]).unwrap();
    }
    let error = fvid::native_plan::concat_wave(&sources).unwrap_err();
    assert!(error.contains("RIFF size"), "{error}");
    let dest = d.0.join("out.wav");
    assert!(fvid::native_pcm::concat_wave(&sources, &dest, None, None).is_err());
    assert!(!dest.exists());
}

#[test]
fn multichannel_identity_preserves_mask_and_rematrix_needs_known_layout() {
    let d = dir();
    for (channels, mask) in [
        (4u16, 0u32),
        (4, 0x33),
        (8, 0x63f),
        (12, 0),
        (32, u32::MAX),
        (64, 0),
    ] {
        let samples: Vec<f32> = (0..137 * usize::from(channels))
            .map(|i| ((i * 29) % 257) as f32 / 128.0 - 1.0)
            .collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let source = d.0.join(format!("{channels}-{mask}.wav"));
        std::fs::write(&source, wave(32, channels, true, Some((32, mask)), &bytes)).unwrap();
        let dest = d.0.join(format!("{channels}-{mask}-out.wav"));
        let stats = export(&source, &dest, None, 0.5, None, None, None, None, None).unwrap();
        assert_eq!(stats.channels, channels);
        assert_eq!(stats.sample_frames, 137);
        let info =
            fvid::native_pcm::inspect(&mut std::fs::File::open(&dest).unwrap(), None).unwrap();
        assert_eq!(info.channel_mask, mask);
        let raw = d.0.join(format!("{channels}-{mask}.f32le"));
        export(&dest, &raw, None, 1.0, None, None, None, None, None).unwrap();
        assert_eq!(
            values(&raw),
            samples.iter().map(|v| v * 0.5).collect::<Vec<_>>()
        );
        let plan = fvid::native_plan::decode_audio(&source, &Default::default()).unwrap();
        assert_eq!(plan.command, "decode-audio");
        let bad = d.0.join(format!("{channels}-{mask}-bad.wav"));
        assert!(export(&source, &bad, None, 1.0, Some(2), None, None, None, None).is_err());
        assert!(!bad.exists());
        let request = fvid::media_info::AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        assert!(fvid::native_plan::decode_audio(&source, &request).is_err());
        {
            let api = d.0.join(format!("{channels}-{mask}-api.f32le"));
            fvid::media::decode_audio(&source, &api, &Default::default()).unwrap();
            assert_eq!(std::fs::read(api).unwrap(), bytes);
        }
    }
}

#[test]
fn wide_resampling_matches_independent_mono_channels() {
    let d = dir();
    let channels = 64usize;
    let count = 113usize;
    let samples: Vec<f32> = (0..count)
        .flat_map(|f| (0..channels).map(move |c| ((f * (c + 1) + c * 7) % 101) as f32 / 64.0 - 0.5))
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let source = d.0.join("wide.wav");
    std::fs::write(&source, wave(32, channels as u16, true, None, &data)).unwrap();
    let destination = d.0.join("wide.f32le");
    let stats = export(
        &source,
        &destination,
        None,
        1.0,
        None,
        Some(11025),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(stats.channels, 64);
    let actual = values(&destination);
    for c in 0..channels {
        let data: Vec<u8> = (0..count)
            .flat_map(|f| samples[f * channels + c].to_le_bytes())
            .collect();
        let source = d.0.join(format!("mono-{c}.wav"));
        std::fs::write(&source, wave(32, 1, true, None, &data)).unwrap();
        let destination = d.0.join(format!("mono-{c}.f32le"));
        let mono = export(
            &source,
            &destination,
            None,
            1.0,
            None,
            Some(11025),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(mono.sample_frames, stats.sample_frames);
        assert_eq!(
            values(&destination),
            actual
                .chunks_exact(channels)
                .map(|frame| frame[c])
                .collect::<Vec<_>>()
        );
    }
}
