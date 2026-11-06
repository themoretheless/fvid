//! Actual paused-first-picture and natural EOF acceptance after a bounded open.
use super::*;
use std::io::Cursor;
#[test]
fn start_bound_presents_the_new_cvs_without_decoding_damaged_old_pictures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-base-dispatch-seek");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut reference = crate::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root.join("healthy-video.mkv")).unwrap()),
        64 << 20,
    )
    .unwrap();
    let mut pictures = Vec::new();
    while let Some(frame) = reference.read_frame_raw().unwrap() {
        pictures.push(frame.into_rgb(64 << 20).unwrap());
    }
    let mut failures = Vec::new();
    for c in manifest["cases"].as_array().unwrap().iter().filter(|c| {
        matches!(
            c["mode"].as_str().unwrap(),
            "healthy" | "header-only" | "bad-escape" | "absent-pps"
        )
    }) {
        let name = c["file"].as_str().unwrap();
        let selected = c["mode"] != "healthy";
        for asynchronous in [false, true] {
            let target = selected.then(|| Duration::from_nanos(c["target_ns"].as_u64().unwrap()));
            let ctx = egui::Context::default();
            let mut p = Player {
                queue: vec![root.join(name)],
                bounds: PlayBounds {
                    start: target,
                    stop: None,
                },
                no_audio: true,
                asynchronous_open: asynchronous,
                start_paused: true,
                ..Default::default()
            };
            p.play_index(0);
            let until = Instant::now() + Duration::from_secs(5);
            while (p.presented.is_none() || p.buffering || p.seek_preview) && p.error.is_none() {
                assert!(
                    Instant::now() < until,
                    "{name}: first frame timeout, async={asynchronous}"
                );
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    p.poll_open(ui.ctx());
                    p.present(ui.ctx());
                });
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            if let Some(error) = &p.error {
                failures.push(format!("{name}: async={asynchronous}: {error}"));
                continue;
            }
            assert_eq!(p.bounds, PlayBounds::default());
            assert!(p.paused);
            assert!(!p.ended);
            let first = if selected {
                c["new_start"].as_u64().unwrap() as usize
            } else {
                0
            };
            assert_eq!(
                display_rgb(p.presented.as_ref().unwrap()).unwrap(),
                pictures[first],
                "{name}: selected first picture"
            );
            p.toggle_pause();
            let until = Instant::now() + Duration::from_secs(5);
            while !p.ended {
                assert!(p.error.is_none(), "{name}: {:?}", p.error);
                assert!(Instant::now() < until, "{name}: EOF timeout");
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    p.poll_open(ui.ctx());
                    p.present(ui.ctx());
                });
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            let last = c["frames"].as_array().unwrap().last().unwrap();
            assert_eq!(
                p.presentation_clock(),
                Some(Duration::from_nanos(last["end_ns"].as_u64().unwrap())),
                "{name}: EOF clock"
            );
            assert_eq!(
                display_rgb(p.presented.as_ref().unwrap()).unwrap(),
                *pictures.last().unwrap(),
                "{name}: terminal picture"
            );
        }
    }
    assert!(
        failures.is_empty(),
        "actual start-bound playback: {failures:?}"
    );
}
