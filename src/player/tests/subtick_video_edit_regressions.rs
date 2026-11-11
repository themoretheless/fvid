//! Drive the actual UI playback state through short movie-clock edit programs.
use super::*;
use std::io::Cursor;
#[test]
fn fractional_movie_edits_reach_the_exact_terminal_clock_and_last_picture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-subtick-video-edits");
    let mut reference = crate::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root.join("unedited.mp4")).unwrap()),
        32 << 20,
    )
    .unwrap();
    let mut colors = Vec::new();
    for _ in 0..3 {
        assert!(reference.read_frame().unwrap());
        colors.push(reference.rgb().to_vec());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let ctx = egui::Context::default();
        let mut player = Player {
            queue: vec![root.join(name)],
            no_audio: true,
            asynchronous_open: false,
            start_paused: true,
            ..Default::default()
        };
        player.play_index(0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while player.presented.is_none() || player.buffering || player.seek_preview {
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            assert!(Instant::now() < deadline, "{name}: first picture timed out");
            let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(1));
        }
        let duration = Duration::from_nanos(c["duration_ns"].as_u64().unwrap());
        assert_eq!(player.duration, Some(duration), "{name}: player duration");
        assert!(player.paused);
        assert!(!player.ended);
        player.toggle_pause();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !player.ended {
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            assert!(Instant::now() < deadline, "{name}: playback did not finish");
            let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.error.is_none(), "{name}: {:?}", player.error);
        assert_eq!(
            player.terminal_clock, None,
            "{name}: natural EOF must not create an explicit stop bound"
        );
        assert_eq!(
            player.presentation_clock(),
            Some(duration),
            "{name}: frozen terminal phase"
        );
        let last = c["frames"].as_array().unwrap().last().unwrap();
        let (start, end, scale) = player.interval.unwrap();
        assert_eq!(
            start * 1_000_000_000 / u128::from(scale),
            u128::from(last["start_ns"].as_u64().unwrap()),
            "{name}: final interval start"
        );
        assert_eq!(
            end * 1_000_000_000 / u128::from(scale),
            u128::from(last["end_ns"].as_u64().unwrap()),
            "{name}: final interval end"
        );
        assert_eq!(
            display_rgb(player.presented.as_ref().unwrap()).unwrap(),
            colors[last["source"].as_u64().unwrap() as usize],
            "{name}: terminal picture"
        );
    }
}
