    fn native_sami_attribute_values_do_not_change_playing_caption_clocks() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/sami-attributes");
        for (name, later) in [("quoted-begin-single", None), ("quoted-class", Some("Вторая"))] {
            let mut player = Player {
                // Y4M presents correctly but deliberately has no indexed seek.
                // Reuse the authored VP9 companion for actual seek acceptance.
                queue: vec![root.parent().unwrap().join("vp9-long-gop-seek.webm")],
                subtitle_file: Some(root.join(format!("{name}.smi"))),
                no_audio: true,
                asynchronous_open: false,
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            assert!(player.playback.is_some());
            let ctx = eframe::egui::Context::default();
            let until = Instant::now() + Duration::from_secs(10);
            while player.presented.is_none() {
                assert!(Instant::now() < until, "{name}: no native video frame");
                let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(player.subtitle_line(Duration::from_millis(500)), None, "{name}");
            assert_eq!(player.subtitle_line(Duration::from_millis(1500)), Some("Строка α"));
            assert_eq!(player.subtitle_line(Duration::from_millis(2500)), None);
            for (target, expected) in [(1, Some("Строка α")), (3, later)] {
                player.seek_time(Duration::from_secs(target));
                let until = Instant::now() + Duration::from_secs(10);
                while player.seek_preview || player.buffering {
                    assert!(Instant::now() < until, "{name}: native seek did not finish");
                    assert!(player.error.is_none(), "{name}: {:?}", player.error);
                    let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                    output.textures_delta.clear();
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(player.subtitle_line(Duration::from_secs(target)), expected, "{name}");
            }
            assert_eq!(player.subtitle_line(Duration::from_secs(5)), None);
        }
    }
