use fvid_media::{CopyOptions, ProgressHook};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn public_adts_normalization_matches_independent_owned_wave_in_both_pass_modes() {
    let dir = Temp(std::env::temp_dir().join(format!("fvid-adts-norm-{}", std::process::id())));
    fs::create_dir_all(&dir.0).unwrap();
    for (index, name) in [
        "audio/aac-mono-44k.aac",
        "audio/aac-stereo.aac",
        "audio/aac-pce-wide8.aac",
        "playback-errors/aac-packet-prefix.aac",
        "playback-errors/aac-loudnorm-long.aac",
    ]
    .into_iter()
    .enumerate()
    {
        let source = fixture(name);
        let bytes = fs::read(&source).unwrap();
        let raw_options = CopyOptions {
            max_packets: if name.contains("loudnorm-long") {
                None
            } else {
                Some(3)
            },
            ..Default::default()
        };
        let reader = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&bytes)).unwrap();
        let mask = fvid_media::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
            .unwrap()
            .channel_mask();
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &raw_options,
        )
        .unwrap();
        if name.contains("loudnorm-long") {
            assert_eq!(
                (stats.decoded_frames, stats.sample_frames),
                (210, 210 * 1024)
            );
            let paired = fixture("playback-errors/aac-loudnorm-long.y4m");
            let video = fvid_media::owned_probe::probe(&paired).unwrap();
            let audio = fvid_media::owned_probe::probe(&source).unwrap();
            assert_eq!(video.streams[0].duration, Some(210));
            assert!((video.duration_us.unwrap() - audio.duration_us.unwrap()).abs() <= 1);
        }
        let wave = dir.0.join(format!("source-{index}.wav"));
        let mut data = fvid_media::owned_wav::float_wav_header_with_mask(
            stats.sample_rate,
            stats.channels,
            stats.sample_frames,
            mask,
        )
        .unwrap();
        data.extend(pcm);
        fs::write(&wave, data).unwrap();
        for dual in [false, true] {
            let args = Some("I=-16:TP=-1.5:LRA=11:print_format=json");
            let actual = dir.0.join(format!("actual-{index}-{dual}.wav"));
            let expected = dir.0.join(format!("expected-{index}-{dual}.wav"));
            let events = Arc::new(Mutex::new(Vec::new()));
            let captured = events.clone();
            let mut options = raw_options.clone();
            options.progress = Some(ProgressHook::new(move |e| captured.lock().unwrap().push(e)));
            let result = if dual {
                fvid_media::apply_loudnorm_dual(&source, &actual, args, &options)
            } else {
                fvid_media::apply_loudnorm(&source, &actual, args, &options)
            }
            .unwrap();
            let expected_stats = if dual {
                fvid_media::owned_loudnorm::apply_loudnorm_dual(
                    &wave,
                    &expected,
                    args,
                    &Default::default(),
                )
            } else {
                fvid_media::owned_loudnorm::apply_loudnorm(
                    &wave,
                    &expected,
                    args,
                    &Default::default(),
                )
            }
            .unwrap();
            assert_eq!(
                fs::read(actual).unwrap(),
                fs::read(expected).unwrap(),
                "{name} dual={dual}"
            );
            assert_eq!(result.backend, expected_stats.backend);
            assert_eq!(result.args, expected_stats.args);
            assert_eq!(result.sample_frames, expected_stats.sample_frames);
            assert_eq!(result.dual_pass, dual);
            let events = events.lock().unwrap();
            assert_eq!(events.iter().filter(|e| e.done).count(), 1);
            assert!(events.last().unwrap().done);
            assert!(
                events
                    .windows(2)
                    .all(|w| w[0].packets <= w[1].packets
                        && w[0].payload_bytes <= w[1].payload_bytes)
            );
        }
    }
}
#[test]
fn owned_adts_plan_is_read_only_and_bad_tail_never_publishes() {
    let source = fixture("playback-errors/aac-packet-prefix.aac");
    let options = CopyOptions {
        max_packets: Some(3),
        progress: Some(ProgressHook::new(|_| panic!("dry run progress"))),
        ..Default::default()
    };
    let plan = fvid_media::plan_loudnorm(&source, None, true, &options).unwrap();
    assert_eq!(plan.command, "loudnorm");
    let report_plan = fvid_media::owned_adts_loudnorm::plan_loudnorm(
        &source,
        Some("I=-16:print_format=json"),
        true,
        &options,
    )
    .unwrap();
    assert!(
        report_plan
            .steps
            .iter()
            .any(|s| s.action == "analyze-output")
    );
    assert!(plan.graph.is_none());
    assert!(plan.steps.iter().any(|s| s.action == "filter"));
    assert!(plan.steps.iter().any(|s| s.action == "write"));
    let dir =
        Temp(std::env::temp_dir().join(format!("fvid-adts-norm-failure-{}", std::process::id())));
    fs::create_dir_all(&dir.0).unwrap();
    let output = dir.0.join("out.wav");
    assert!(
        fvid_media::owned_adts_loudnorm::apply(&source, &output, None, false, &Default::default())
            .unwrap_err()
            .contains("fill whole buffer")
    );
    assert!(!output.exists());
    let options = CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_adts_loudnorm::apply(&source, &output, None, false, &options).is_err()
    );
    assert!(!output.exists());
    fs::write(&output, b"preserve").unwrap();
    assert!(
        fvid_media::owned_adts_loudnorm::apply(&source, &output, None, true, &Default::default())
            .unwrap_err()
            .contains("already exists")
    );
    assert_eq!(fs::read(output).unwrap(), b"preserve");
}
