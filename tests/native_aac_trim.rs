use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-aac-trim-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn trim_keeps_decoder_preroll_and_matches_full_decode_sample_slice() {
    let d = dir("samples");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
    ] {
        let source = fixture(name);
        let data = std::fs::read(&source).unwrap();
        let mut full = Vec::new();
        let original =
            fvid::native_media::decode_aac_pcm(&data, &mut full, &Default::default()).unwrap();
        for (from, to) in [(33333, 100001), (1, 2000000)] {
            let dest = d.0.join(format!("{name}-{from}.wav"));
            let stats =
                fvid::native_export::trim_adts_wave(&source, &dest, from, to, None, None).unwrap();
            let first = (from as u64 * u64::from(original.sample_rate))
                .div_ceil(1000000)
                .min(original.sample_frames);
            let last = (to as u64 * u64::from(original.sample_rate))
                .div_ceil(1000000)
                .min(original.sample_frames);
            assert_eq!(stats.sample_frames, last - first);
            let bytes = std::fs::read(&dest).unwrap();
            let stride = usize::from(original.channels) * 4;
            assert_eq!(
                &bytes[80..],
                &full[first as usize * stride..last as usize * stride]
            );
            let cli = d.0.join(format!("cli-{name}-{from}.wav"));
            let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "trim"])
                .arg(&source)
                .arg(&cli)
                .args([
                    "--from",
                    &format!("{}.{:06}", from / 1000000, from % 1000000),
                    "--to",
                    &format!("{}.{:06}", to / 1000000, to % 1000000),
                    "--streams",
                    "0",
                ])
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(std::fs::read(cli).unwrap(), bytes);
            let plan = fvid::native_plan::trim_adts(&source, from, to).unwrap();
            assert!(
                plan.steps
                    .last()
                    .unwrap()
                    .detail
                    .contains(&format!("{} float32", last - first))
            );
            {
                let out = d.0.join(format!("api-{name}-{from}.wav"));
                let stats =
                    fvid::media::trim(&source, &out, from, to, &Default::default()).unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(std::fs::read(out).unwrap(), bytes);
                assert_eq!(
                    fvid::media::plan_trim(&source, from, to, &Default::default()).unwrap(),
                    plan
                );
            }
        }
    }
}
#[test]
fn invalid_ranges_cancel_and_existing_outputs_do_not_publish() {
    let d = dir("invalid");
    let source = fixture("aac-mono-44k.aac");
    let dest = d.0.join("out.wav");
    for (from, to) in [(-1, 1000), (1000, 1000), (2000000, 3000000)] {
        assert!(fvid::native_export::trim_adts_wave(&source, &dest, from, to, None, None).is_err());
        assert!(fvid::native_plan::trim_adts(&source, from, to).is_err());
        assert!(!dest.exists());
    }
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 0 {
            stop.cancel();
        }
    });
    assert!(
        fvid::native_export::trim_adts_wave(&source, &dest, 1000, 90000, Some(&flag), Some(&hook))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
    std::fs::write(&dest, b"keep").unwrap();
    assert!(fvid::native_export::trim_adts_wave(&source, &dest, 1000, 90000, None, None).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "trim"])
        .arg(&source)
        .args(["--from", "0.033333", "--to", "0.100001"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(),
        serde_json::to_value(fvid::native_plan::trim_adts(&source, 33333, 100001).unwrap())
            .unwrap()
    );
}

#[test]
fn selected_container_audio_keeps_timing_and_rejects_implicit_track_loss() {
    let d = dir("containers");
    for (name, stream, reference) in [
        (
            "two-audio.mp4",
            1,
            include_bytes!("fixtures/audio/two-audio-stream1-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mp4",
            2,
            include_bytes!("fixtures/audio/two-audio-stream2-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mka",
            0,
            include_bytes!("fixtures/audio/two-audio-mka0-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mka",
            1,
            include_bytes!("fixtures/audio/two-audio-mka1-reference.f32le").as_slice(),
        ),
    ] {
        let source = fixture(name);
        let dest = d.0.join(format!("{name}-{stream}.wav"));
        assert!(
            fvid::native_export::trim_aac_wave(&source, &dest, 0, 40000, None, None, None).is_err()
        );
        assert!(fvid::native_plan::trim_aac(&source, 0, 40000, None).is_err());
        assert!(!dest.exists());
        let stats =
            fvid::native_export::trim_aac_wave(&source, &dest, 0, 40000, Some(stream), None, None)
                .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let pcm = &bytes[80..];
        assert_eq!(pcm.len(), reference.len());
        let mut square = 0.0f64;
        let mut peak = 0.0f64;
        for (actual, expected) in pcm.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let delta = f64::from(f32::from_le_bytes(*actual))
                - f64::from(f32::from_le_bytes(*expected));
            square += delta * delta;
            peak = peak.max(delta.abs());
        }
        assert!(peak < 5e-4 && (square / (pcm.len() / 4) as f64).sqrt() < 1e-4);
        assert_eq!(
            stats.sample_frames,
            u64::from(stats.sample_rate) * 40 / 1000
        );
        let cli = d.0.join(format!("cli-{name}-{stream}.wav"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "trim"])
            .arg(&source)
            .arg(&cli)
            .args([
                "--from",
                "0",
                "--to",
                "0.04",
                "--streams",
                &stream.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), bytes);
        let plan = fvid::native_plan::trim_aac(&source, 0, 40000, Some(stream)).unwrap();
        assert_eq!(plan.streams[0].index, stream);
        {
            let options = fvid::media::CopyOptions {
                streams: vec![stream],
                ..Default::default()
            };
            let api = d.0.join(format!("api-{name}-{stream}.wav"));
            assert!(fvid::media::trim(&source, &api, 0, 40000, &Default::default()).is_err());
            fvid::media::trim(&source, &api, 0, 40000, &options).unwrap();
            assert_eq!(std::fs::read(api).unwrap(), bytes);
            assert_eq!(
                fvid::media::plan_trim(&source, 0, 40000, &options).unwrap(),
                plan
            );
        }
    }
    let source = fixture("two-audio.mp4");
    assert!(
        fvid::native_export::trim_aac_wave(
            &source,
            &d.0.join("video.wav"),
            0,
            40000,
            Some(0),
            None,
            None
        )
        .is_err()
    );
    assert!(fvid::native_plan::trim_aac(&source, 0, 40000, Some(9)).is_err());
}

#[test]
fn single_container_track_can_be_trimmed_without_selection() {
    let d = dir("single");
    for name in [
        "aac-native-edit.m4a",
        "aac-stereo.mka",
        "aac-960-48000.m4a",
        "aac-960-48000.mka",
    ] {
        let source = fixture(name);
        let dest = d.0.join(format!("{name}.wav"));
        let baseline = d.0.join(format!("{name}.f32le"));
        fvid::native_export::export_aac_pcm_selected(
            &source,
            &baseline,
            Some((
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(30),
            )),
            1.0,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        fvid::native_export::trim_aac_wave(&source, &dest, 10000, 30000, None, None, None).unwrap();
        assert_eq!(
            &std::fs::read(dest).unwrap()[80..],
            std::fs::read(baseline).unwrap()
        );
    }
}
