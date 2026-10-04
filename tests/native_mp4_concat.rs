use fvid::{
    container::{mp4, webm},
    media_control::{CancelFlag, ProgressHook},
    native_export,
    native_geometry::VideoGeometry,
    playback_native::NativeReader,
};
use std::{
    fs::File,
    io::BufReader,
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
    let p = std::env::temp_dir().join(format!("fvid-mp4-concat-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn pixels(path: &Path) -> Vec<Vec<u8>> {
    let mut reader =
        NativeReader::software(BufReader::new(File::open(path).unwrap()), usize::MAX).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        frames.push(
            VideoGeometry::default()
                .apply_display(&frame, w, h, reader.rotation())
                .unwrap()
                .data,
        );
    }
    frames
}
#[test]
fn avc_hevc_payloads_and_decoded_frames_repeat_with_contiguous_clock() {
    let d = dir("video");
    for (index, name) in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"]
        .iter()
        .enumerate()
    {
        let source = fixture(name);
        let output = d.0.join(format!("video-{index}.mkv"));
        let inputs = vec![source.clone(), source.clone()];
        let event = native_export::try_concat_mp4_matroska(&inputs, &output, None, None)
            .unwrap()
            .expect("owned concat admission");
        assert!(event.done);
        let expected = pixels(&source);
        let actual = pixels(&output);
        assert_eq!(
            actual,
            expected
                .iter()
                .chain(expected.iter())
                .cloned()
                .collect::<Vec<_>>()
        );
        let mut mp4 = mp4::Mp4Reader::open(
            BufReader::new(File::open(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut mkv = webm::WebmReader::open(
            BufReader::new(File::open(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        mkv.scan_all().unwrap();
        assert_eq!(mkv.tracks.len(), mp4.tracks().len());
        let mut payload = Vec::new();
        for track in 0..mp4.tracks().len() {
            let packets = mkv
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == mkv.tracks[track].number)
                .map(|(i, p)| (i, p.pts_ns, p.duration_ns.unwrap(), p.invisible))
                .collect::<Vec<_>>();
            assert_eq!(packets.len() % 2, 0);
            let half = packets.len() / 2;
            let span = packets[..half]
                .iter()
                .filter(|p| !p.3)
                .map(|p| p.1 + p.2 as i64)
                .max()
                .unwrap();
            for i in 0..half {
                mp4.read_packet(track, i, &mut payload).unwrap();
                assert_eq!(mkv.read_packet(packets[i].0).unwrap(), payload);
                assert_eq!(mkv.read_packet(packets[i + half].0).unwrap(), payload);
                assert_eq!(packets[i + half].1 - packets[i].1, span);
                assert_eq!(packets[i + half].2, packets[i].2);
            }
        }

    }
}
#[test]
fn zero_delay_aac_matches_the_concatenated_coded_sequence() {
    let d = dir("aac");
    let source = d.0.join("source.m4a");
    native_export::remux_adts_aac(&fixture("audio/aac-stereo.aac"), &source).unwrap();
    let output = d.0.join("concat.mka");
    assert!(
        native_export::try_concat_mp4_matroska(
            &[source.clone(), source.clone()],
            &output,
            None,
            None
        )
        .unwrap()
        .is_some()
    );
    let raw = std::fs::read(fixture("audio/aac-stereo.aac")).unwrap();
    let reference = d.0.join("sequence.aac");
    std::fs::write(&reference, [raw.as_slice(), raw.as_slice()].concat()).unwrap();
    let expected = d.0.join("expected.wav");
    let combined = d.0.join("combined.wav");
    native_export::export_aac_pcm(&reference, &expected).unwrap();
    native_export::export_aac_pcm(&output, &combined).unwrap();
    let a = std::fs::read(expected).unwrap();
    let b = std::fs::read(combined).unwrap();
    assert_eq!(a.len(), b.len());
    assert!(a == b, "coded-sequence PCM mismatch");

}
#[test]
fn cancellation_no_overwrite_and_incompatible_inputs_preserve_publication() {
    let d = dir("cancel");
    let source = fixture("video.mp4");
    let output = d.0.join("out.mkv");
    let flag = CancelFlag::new();
    let captured = flag.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let saved = events.clone();
    let hook = ProgressHook::new(move |event| {
        saved.lock().unwrap().push(event);
        if event.packets > 0 {
            captured.cancel();
        }
    });
    assert!(
        native_export::try_concat_mp4_matroska(
            &[source.clone(), source.clone()],
            &output,
            Some(&flag),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
    assert!(events.lock().unwrap().iter().all(|e| !e.done));
    assert!(
        native_export::try_concat_mp4_matroska(
            &[source.clone(), fixture("hevc/main-ipb.mp4")],
            &output,
            None,
            None
        )
        .unwrap()
        .is_none()
    );
    std::fs::write(&output, b"existing").unwrap();
    assert!(
        native_export::try_concat_mp4_matroska(&[source.clone(), source], &output, None, None)
            .is_err()
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"existing");
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
#[test]
fn public_concat_and_plan_select_owned_matroska_workflow() {
    let d = dir("api");
    let source = fixture("hevc/main-ipb.mp4");
    let inputs = vec![source.clone(), source];
    let out = d.0.join("out.mkv");
    let plan = fvid::media::plan_concat(&inputs, &Default::default()).unwrap();
    assert!(plan.notes.iter().any(|n| n.contains("backend: fvid")));
    let stats = fvid::media::concat(&inputs, &out, &Default::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.segments, 2);
}



#[test]
fn per_segment_aac_priming_is_transported_without_rewriting_packets() {
    let d = dir("primed");
    let source = fixture("audio/aac-native-edit.m4a");
    let out = d.0.join("out.mka");
    let stats =
        native_export::try_concat_mp4_matroska(&[source.clone(), source.clone()], &out, None, None)
            .unwrap()
            .expect("primed AAC admission");
    assert!(stats.done);
    let mut mkv = webm::WebmReader::open(
        BufReader::new(File::open(&out).unwrap()),
        Default::default(),
    )
    .unwrap();
    mkv.scan_all().unwrap();
    assert!(mkv.tracks[0].codec_delay_ns > 0);
    assert!(mkv.packets.iter().any(|p| p.discard_padding_ns < 0));
    let mut original = mp4::Mp4Reader::open(
        BufReader::new(File::open(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let half = mkv.packets.len() / 2;
    let mut data = Vec::new();
    for index in 0..half {
        original.read_packet(0, index, &mut data).unwrap();
        assert!(mkv.read_packet(index).unwrap() == data);
        assert!(mkv.read_packet(index + half).unwrap() == data);
    }
    let pcm = d.0.join("combined.wav");
    let first = d.0.join("first.wav");
    let expected = native_export::export_aac_pcm(&source, &first).unwrap();
    let actual = native_export::export_aac_pcm(&out, &pcm).unwrap();
    assert_eq!(actual.sample_frames, expected.sample_frames * 2);

}

#[test]
fn cli_concat_runs_owned_without_the_media_feature() {
    let d = dir("cli");
    let source = fixture("hevc/main10-ipb.mp4");
    let output = d.0.join("out.mkv");
    let plan = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "concat"])
        .arg(&source)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert!(
        json["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("backend: fvid"))
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&output)
        .arg(&source)
        .arg(&source)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert_eq!(stats["segments"], 2);
    let events = String::from_utf8(result.stderr).unwrap();
    let completed = events
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["done"] == true)
        .count();
    assert_eq!(completed, 1);
    let expected = pixels(&source);
    assert_eq!(pixels(&output), [expected.clone(), expected].concat());
}
