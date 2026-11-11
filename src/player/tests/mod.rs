use super::*;

#[cfg(test)]
mod tests {
    use super::{
        ADJUST_IDENTITY, ASPECTS, Adjust, Aspect, CROPS, ChapterMark, Control, FileTags, Frame,
        Grading, HIDE_AFTER, LoopMark, NO_CROP, PANEL_NITS, Panel, PathBuf, Pixels, Planar8,
        PlayArgs, PlayBounds, Player, Pos2, RATES, Rect, Repeat, SubtitleSource, VOLUME_MAX, Vec2,
        adjust_luma, adjust_rgb, adjust_scalars, advance, aspect_label, aspect_osd, aspect_step,
        bitrate_text, buffer_text, buffered_fraction, byte_size, chapter_ahead, chapter_at,
        colour_line, container_facts, covered_span, crop_insets, crop_label, crop_osd, crop_step,
        cropped_size, cycle_repeat, deal_cycle, delay_step, delayed_clock, display_size, end_clock,
        expand_inputs, file_size, fps_text, grade_text, jump_size, light_line, loop_press,
        loop_rewind, next_frame_deadline, paced_period, parse_clock, parse_play_args, playlist_osd,
        position_from_digit, rate_fine, rate_osd, rate_step, repeat_osd, retreat, shown_insets,
        shown_size, snapshot_name, sound_codec, spool_text, subtitles, track_step, uv_window,
        video_rect, volume_osd, volume_step, zoom_osd, zoom_step,
    };
    use super::{Event, NativeReader, Playback};
    use crate::color::{
        ColourDescription, ContentLight, DisplayTarget, HdrMetadata, Interpolation, Log, Lut,
        MasteringDisplay, Primaries, ToneMap, Transfer,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// `fvid play` sees strings; the tests write words.
    fn play_args(words: &[&str]) -> crate::Result<PlayArgs> {
        parse_play_args(
            &words
                .iter()
                .map(|word| word.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn timeline_animation_expands_reverses_and_settles() {
        use eframe::egui;
        let ctx = egui::Context::default();
        let id = egui::Id::new("timeline-animation-test");
        let mut value = 0.0f32;
        let mut time = 0.0;
        let mut step = |target: f32| {
            time += 1.0 / 60.0;
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                value = super::smooth_timeline_value(ui.ctx(), id, target, 0.0, 0.120);
            });
            output.textures_delta.clear();
            value
        };
        let first = step(1.0);
        let second = step(1.0);
        assert!(first > 0.0 && second > first && second < 1.0);
        let reversed = step(0.0);
        assert!(reversed > 0.0 && reversed < second);
        for _ in 0..180 { step(0.0); }
        assert_eq!(step(0.0), 0.0);
        for _ in 0..180 { step(1.0); }
        assert_eq!(step(1.0), 1.0);
    }

    #[test]
    fn timeline_animation_starts_smoothly_after_idle() {
        use eframe::egui;
        let ctx = egui::Context::default();
        let id = egui::Id::new("timeline-after-idle");
        let mut previous = 0.0f32;
        for (time, target) in [(0.0, 0.0), (10.0, 1.0), (20.0, 0.0)] {
            let mut value = 0.0;
            let mut output = ctx.run_ui(egui::RawInput {
                time: Some(time), ..Default::default()
            }, |ui| {
                value = super::smooth_timeline_value(ui.ctx(), id, target, 0.0, 0.120);
            });
            output.textures_delta.clear();
            if time > 0.0 {
                assert!(value > previous.min(target) && value < previous.max(target));
                assert!((value - previous).abs() < (target - previous).abs() * 0.3,
                    "idle time must not advance a newly requested transition");
            }
            previous = value;
        }
    }

    #[test]
    fn timeline_animation_has_equal_speed_across_refresh_rates() {
        use eframe::egui;
        fn sample(hz: usize) -> f32 {
            let ctx = egui::Context::default();
            let id = egui::Id::new("timeline-refresh-rate");
            let mut value = 0.0;
            for frame in 0..=hz / 2 {
                let mut output = ctx.run_ui(egui::RawInput {
                    time: Some(frame as f64 / hz as f64),
                    ..Default::default()
                }, |ui| {
                    value = super::smooth_timeline_value(
                        ui.ctx(), id, if frame == 0 { 0.0 } else { 1.0 }, 0.0, 0.120,
                    );
                });
                output.textures_delta.clear();
            }
            value
        }
        let slow = sample(30);
        for hz in [60, 120, 144] {
            assert!((slow - sample(hz)).abs() < 0.00001, "refresh rate {hz}");
        }
    }

    #[test]
    fn timeline_animation_position_initializes_exactly_then_tracks_new_targets() {
        use eframe::egui;
        let ctx = egui::Context::default();
        let id = egui::Id::new("timeline-position-test");
        let mut value = 0.0f32;
        for (frame, target) in [0.2f32, 0.8, 0.1].into_iter().enumerate() {
            let before = value;
            let mut output = ctx.run_ui(egui::RawInput {
                time: Some(frame as f64 / 60.0), ..Default::default()
            }, |ui| { value = super::smooth_timeline_position(ui.ctx(), id, target); });
            output.textures_delta.clear();
            if frame == 0 { assert_eq!(value, target); }
            else { assert!(value > before.min(target) && value < before.max(target)); }
        }
    }

    #[test]
    fn spherical_arrow_is_consumed_before_transport_seek() {
        use eframe::egui::{self, Event, Key, Modifiers, RawInput};
        for enabled in [false, true] {
            let ctx = egui::Context::default();
            let input = RawInput { events: vec![Event::Key {
                key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE,
            }], ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let look = super::spherical_look_pressed(ui.ctx(), enabled);
                assert_eq!(look, enabled.then_some((5000,0,0)));
                let transport = super::controls_pressed(ui.ctx());
                assert_eq!(transport.iter().any(|c| matches!(c, Control::Jump(_, true))), !enabled);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn spherical_look_on_pause_preserves_clock_and_bounds_angles() {
        let rgb: Vec<u8> = (0..96).map(|n| (n * 2) as u8).collect();
        let frame = Frame { pixels: Pixels::Rgb(rgb), dimensions: [8,4], period: Duration::from_millis(17),
            interval: Some((2,3,60)), pts: Some((2,60)), generation: 7, serial: 9, rotation: 0 };
        let mut player = Player { paused: true, presented_source: Some(frame.clone()), presented: Some(frame), ..Default::default() };
        player.spherical.enabled = true;
        player.redraw_view();
        let before = super::display_rgb(player.presented.as_ref().unwrap()).unwrap();
        player.look_spherical(90_000, 0, 0);
        let after = super::display_rgb(player.presented.as_ref().unwrap()).unwrap();
        assert_ne!(before, after);
        assert_eq!(player.presented.as_ref().unwrap().pts, Some((2,60)));
        assert!(player.paused);
        player.look_spherical(i32::MAX, i32::MAX, i32::MIN);
        assert!((-180_000..180_000).contains(&player.spherical.yaw));
        assert!((-180_000..180_000).contains(&player.spherical.roll));
        assert_eq!(player.spherical.pitch, 90_000);
    }

    #[test]
    fn spherical_toggle_redraws_a_paused_frame_from_original_codes() {
        let rgb: Vec<u8> = (0..96).map(|n| (n * 2) as u8).collect();
        let frame = Frame { pixels: Pixels::Rgb(rgb.clone()), dimensions: [8,4],
            period: Duration::from_millis(17), interval: Some((2,3,60)),
            pts: Some((2,60)), generation: 7, serial: 9, rotation: 0 };
        let mut player = Player { paused: true, presented_source: Some(frame.clone()), presented: Some(frame), ..Default::default() };
        player.change_spherical(false);
        let Pixels::Rgb(projected) = &player.presented.as_ref().unwrap().pixels else { panic!("expected RGB") };
        assert_ne!(projected, &rgb);
        player.change_spherical(false);
        let Pixels::Rgb(restored) = &player.presented.as_ref().unwrap().pixels else { panic!("expected RGB") };
        assert_eq!(restored, &rgb);
        assert_eq!(player.presented.as_ref().unwrap().pts, Some((2,60)));
        assert!(player.paused);
    }

    #[test]
    fn native_media_hdr_operator_names_reach_the_owned_grade() {
        let signal = ColourDescription { primaries: 9, transfer: 16, matrix: 9, full_range: false };
        for mode in ["off", "clip", "reinhard", "hable", "mobius", "aces", "maxrgb"] {
            let parsed = super::parse_media_play_args(&[format!("--hdr-tonemap={mode}"), "clip.mp4".into()]).unwrap();
            let grade = parsed.grading.grade_for(signal, &HdrMetadata::default()).unwrap();
            assert_eq!(grade.plan().tone_map.unwrap().label(), mode);
        }
        assert!(parse_play_args(&["--hdr-tonemap=bad".into()]).is_err());
    }

    #[test]
    fn native_hdr_mastering_preserves_corners_and_changes_content_peak() {
        let parsed = super::parse_media_play_args(&["--hdr-mastering=0.005,4000".into(), "clip.mp4".into()]).unwrap();
        let signal = ColourDescription { primaries: 9, transfer: 16, matrix: 9, full_range: false };
        let original = MasteringDisplay { red: Primaries::BT709.r, green: Primaries::BT709.g, blue: Primaries::BT709.b, white: Primaries::BT709.white, min_luminance: 0.1, max_luminance: 1000.0 };
        let metadata = parsed.grading.metadata_for(signal, &HdrMetadata { mastering: Some(original), light: ContentLight::default() });
        let volume = metadata.mastering.unwrap();
        assert_eq!(volume.red, original.red);
        assert_eq!(volume.green, original.green);
        assert_eq!(volume.blue, original.blue);
        assert_eq!(volume.white, original.white);
        assert_eq!(volume.min_luminance, 0.005);
        assert_eq!(metadata.content_light(100.0).max_cll, 4000.0);
        let mut high = vec![200,180,160];
        let mut low = high.clone();
        parsed.grading.grade_for(signal, &HdrMetadata::default()).unwrap().apply(&mut high);
        Grading::default().grade_for(signal, &HdrMetadata { mastering: Some(original), light: ContentLight::default() }).unwrap().apply(&mut low);
        assert_ne!(high, low);
        assert!(parse_play_args(&["--hdr-mastering=10,1".into()]).is_err());
    }

    #[test]
    fn native_hdr_display_and_content_limits_change_highlight_mapping() {
        let parse = |nits: &str, cll: &str| super::parse_media_play_args(&[
            format!("--hdr-nits={nits}"), "--hdr-maxcll".into(), cll.into(), "clip.mp4".into()
        ]).unwrap();
        let signal = ColourDescription { primaries: 9, transfer: 16, matrix: 9, full_range: false };
        let hdr = HdrMetadata::default();
        let apply = |grading: &Grading| {
            let mut rgb = vec![200,180,160,240,230,220];
            grading.grade_for(signal, &hdr).unwrap().apply(&mut rgb);
            rgb
        };
        let low = parse("100", "4000,1000");
        let high = parse("400", "4000,1000");
        assert_ne!(apply(&low.grading), apply(&high.grading), "display luminance must affect HDR mapping");
        let other_content = parse("100", "1000,400");
        assert_ne!(apply(&low.grading), apply(&other_content.grading), "content peak must affect the shoulder");
        assert_eq!(low.grading.content_light.unwrap().max_fall, 1000.0);
        assert!(parse_play_args(&["--hdr-nits=0".into()]).is_err());
        assert!(parse_play_args(&["--hdr-maxcll=bad".into()]).is_err());
    }

    #[test]
    fn spherical_stereo_never_samples_the_other_eye() {
        for layout in ["sbs", "tb"] {
            let parsed = super::parse_media_play_args(&[format!("--spherical-stereo={layout}"), "clip.y4m".into()]).unwrap();
            assert!(parsed.spherical.enabled);
            let rgb: Vec<u8> = (0..4).flat_map(|y| (0..8).flat_map(move |x| {
                if (layout == "sbs" && x < 4) || (layout == "tb" && y < 2) { [20,40,60] } else { [200,180,160] }
            })).collect();
            let mut frame = Frame {
                pixels: Pixels::Rgb(rgb), dimensions: [8,4], period: Duration::from_millis(17),
                interval: Some((2,3,60)), pts: Some((2,60)), generation: 7, serial: 9, rotation: 0,
            };
            super::spherical_frame(&mut frame, parsed.spherical).unwrap();
            let Pixels::Rgb(rgb) = &frame.pixels else { panic!("expected RGB") };
            assert_eq!(rgb, &vec![20,40,60].repeat(32));
            assert_eq!(frame.dimensions, [8,4]);
            assert_eq!(frame.interval, Some((2,3,60)));
            assert_eq!(frame.pts, Some((2,60)));
        }
        assert!(parse_play_args(&["--spherical-stereo=bad".into()]).is_err());
    }

    #[test]
    fn native_spherical_projection_uses_owned_mapping_and_keeps_clock() {
        let parsed = super::parse_media_play_args(&["--spherical-projection=equirect".into(), "--yaw=90".into(), "--fov=80".into(), "clip.y4m".into()]).unwrap();
        assert!(parsed.spherical.enabled);
        assert_eq!(parsed.spherical.yaw, 90_000);
        let source: Vec<u32> = (0..32).map(|n| n * 0x070503).collect();
        let mut frame = Frame {
            pixels: Pixels::Rgb(source.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect()),
            dimensions: [8,4], period: Duration::from_millis(17), interval: Some((2,3,60)),
            pts: Some((2,60)), generation: 7, serial: 9, rotation: 0,
        };
        super::spherical_frame(&mut frame, parsed.spherical).unwrap();
        let original: Vec<u8> = source.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        let Pixels::Rgb(rgb) = &frame.pixels else { panic!("expected RGB") };
        assert_eq!(rgb.len(), original.len());
        assert_ne!(rgb, &original, "the viewing direction must change the image");
        assert_eq!(frame.pts, Some((2,60)));
        assert_eq!(frame.dimensions, [8,4]);
        assert!(parse_play_args(&["--spherical-projection=bad".into()]).is_err());
    }

    #[test]
    fn native_stereo_preserves_timing_and_selects_eye_channels() {
        let parsed = super::parse_media_play_args(&["--play-stereo3d=sbsl".into(), "clip.y4m".into()]).unwrap();
        assert_eq!(parsed.stereo3d, fvid_media::owned_spherical::PlayStereo3D::SbslAnaglyph);
        assert!(parse_play_args(&["--play-stereo3d=bad".into()]).is_err());
        let mut frame = Frame {
            pixels: Pixels::Rgb(vec![10,20,30,40,50,60,70,80,90,100,110,120]),
            dimensions: [4,1], period: Duration::from_millis(17),
            interval: Some((2,3,60)), pts: Some((2,60)), generation: 7, serial: 9, rotation: 0,
        };
        super::stereo_frame(&mut frame, parsed.stereo3d).unwrap();
        assert_eq!(frame.dimensions, [2,1]);
        let Pixels::Rgb(rgb) = &frame.pixels else { panic!("expected RGB") };
        assert_eq!(rgb, &[10,80,90,40,110,120]);
        assert_eq!(frame.pts, Some((2,60)));
        assert_eq!(frame.interval, Some((2,3,60)));
        assert_eq!(frame.serial, 9);
        assert_eq!(frame.generation, 7);
        super::stereo_frame(&mut frame, fvid_media::owned_spherical::PlayStereo3D::Off).unwrap();
        assert_eq!(frame.dimensions, [2,1]);
    }

    #[test]
    fn media_play_indices_keep_their_zero_based_meaning() {
        let words: Vec<String> = ["--audio-track=0", "--subtitle-track", "1", "clip.mp4"].into_iter().map(str::to_owned).collect();
        let parsed = super::parse_media_play_args(&words).unwrap();
        assert_eq!(parsed.audio_track, Some(0));
        assert_eq!(parsed.embedded_subtitle, Some(1));
        let literal = super::parse_media_play_args(&["--subtitles".into(), "--audio-track".into(), "clip.mp4".into()]).unwrap();
        assert_eq!(literal.subtitle_file, Some(PathBuf::from("--audio-track")));

        let off = super::parse_media_play_args(&["--subtitle-track=-1".into(), "clip.mp4".into()]).unwrap();
        assert!(off.no_subtitles);
        assert!(super::parse_media_play_args(&["--subtitle-track=-2".into()]).is_err());
    }

    #[test]
    fn embedded_selection_is_not_shifted_by_a_sidecar_and_can_start_hidden() {
        let directory = scratch("fvid-media-subtitle-selection", &[]);
        let path = directory.join("clip.mkv");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv")).unwrap();
        std::fs::write(directory.join("clip.srt"), "1\n00:00:00,000 --> 00:00:02,000\nsidecar\n").unwrap();
        let mut player = Player { preferred_embedded_subtitle: Some(0), subtitle_shown: false, ..Default::default() };
        player.load_subtitles(&path);
        assert_eq!(player.subtitle_source, 1);
        assert!(player.subtitle_line(Duration::from_millis(700)).is_none());
        player.apply(Control::Subtitles);
        assert_eq!(player.subtitle_line(Duration::from_millis(700)), Some("plain first"));
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn eac3_stereo_cli_is_session_scoped_and_validated() {
        use crate::codec::eac3_decoder::StereoDownmixMode as Mode;
        assert_eq!(play_args(&["clip.y4m"]).unwrap().eac3_stereo,Some(Mode::Automatic));
        let policy=Player::default().dolby_policy;
        assert!(policy.eac3_stereo_output);
        assert_eq!(policy.eac3_stereo_downmix.mode,Mode::Automatic);
        assert!(policy.eac3_stereo_downmix.headroom);
        // A library worker can still feed a native multichannel backend.
        assert!(!crate::audio_thread::DolbyPlaybackPolicy::default().eac3_stereo_output);
        for (value,wanted) in [("native",None),("loro",Some(Mode::LoRo)),("ltrt",Some(Mode::LtRt)),("auto",Some(Mode::Automatic))] {
            for words in [vec![format!("--eac3-stereo={value}"),"clip.y4m".into()],vec!["--eac3-stereo".into(),value.into(),"clip.y4m".into()]] {
                assert_eq!(super::parse_play_args(&words).unwrap().eac3_stereo,wanted);
                assert_eq!(super::parse_media_play_args(&words).unwrap().eac3_stereo,wanted);
            }
        }
        for words in [vec!["--eac3-stereo"],vec!["--eac3-stereo=bad","clip.y4m"],vec!["--eac3-stereo=","clip.y4m"]] {assert!(play_args(&words).is_err());}
    }

    #[test]
    fn ac3_stereo_cli_is_session_scoped_and_validated() {
        use crate::codec::ac3_decoder::StereoDownmixMode as Mode;
        assert_eq!(play_args(&["clip.y4m"]).unwrap().ac3_stereo,Some(Mode::Automatic));
        let policy=Player::default().dolby_policy;
        assert!(policy.ac3_stereo_output);
        assert_eq!(policy.ac3_stereo_downmix.mode,Mode::Automatic);
        assert!(policy.ac3_stereo_downmix.headroom);
        // A library worker can still feed a native multichannel backend.
        assert!(!crate::audio_thread::DolbyPlaybackPolicy::default().ac3_stereo_output);
        for (value,wanted) in [("native",None),("loro",Some(Mode::LoRo)),("ltrt",Some(Mode::LtRt)),("auto",Some(Mode::Automatic))] {
            for words in [vec![format!("--ac3-stereo={value}"),"clip.y4m".into()],vec!["--ac3-stereo".into(),value.into(),"clip.y4m".into()]] {
                assert_eq!(super::parse_play_args(&words).unwrap().ac3_stereo,wanted);
                assert_eq!(super::parse_media_play_args(&words).unwrap().ac3_stereo,wanted);
            }
        }
        for words in [vec!["--ac3-stereo"],vec!["--ac3-stereo=bad","clip.y4m"],vec!["--ac3-stereo=","clip.y4m"]] {assert!(play_args(&words).is_err());}
    }

    #[test]
    fn dolby_compression_cli_is_session_scoped_and_validated() {
        use crate::codec::ac3_decoder::CompressionMode;
        assert_eq!(play_args(&["clip.y4m"]).unwrap().dolby_compression, CompressionMode::Line);
        for words in [vec!["--dolby-compression=rf", "clip.y4m"],vec!["--dolby-compression", "rf", "clip.y4m"]] {
            assert_eq!(play_args(&words).unwrap().dolby_compression,CompressionMode::Rf);
            let args=words.iter().map(|s|s.to_string()).collect::<Vec<_>>();
            assert_eq!(super::parse_media_play_args(&args).unwrap().dolby_compression,CompressionMode::Rf);
        }
        for words in [vec!["--dolby-compression"],vec!["--dolby-compression=off","clip.y4m"],vec!["--dolby-compression=","clip.y4m"]] {
            assert!(play_args(&words).is_err());
        }
    }

    #[test]
    fn dolby_dialogue_normalization_cli_is_session_scoped_and_validated() {
        assert!(!play_args(&["clip.y4m"]).unwrap().dolby_dialogue_normalization);
        for (value,wanted) in [("on",true),("off",false)] {
            for words in [vec![format!("--dolby-dialogue-normalization={value}"),"clip.y4m".into()],vec!["--dolby-dialogue-normalization".into(),value.into(),"clip.y4m".into()]] {
                let parsed=super::parse_play_args(&words).unwrap();assert_eq!(parsed.dolby_dialogue_normalization,wanted);
                assert_eq!(super::parse_media_play_args(&words).unwrap().dolby_dialogue_normalization,wanted);
            }
        }
        for words in [vec!["--dolby-dialogue-normalization"],vec!["--dolby-dialogue-normalization=rf","clip.y4m"],vec!["--dolby-dialogue-normalization=","clip.y4m"]] {
            assert!(play_args(&words).is_err());
        }
    }

    #[test]
    fn native_audio_device_options_work_without_an_input_or_window() {
        let parsed = play_args(&["--list-audio-devices"]).unwrap();
        assert!(parsed.list_audio_devices);
        assert!(parsed.paths.is_empty());
        let parsed = play_args(&["--audio-device=USB DAC", "clip.y4m"]).unwrap();
        assert_eq!(parsed.audio_device.as_deref(), Some("USB DAC"));
        assert!(play_args(&["--audio-device=", "clip.y4m"]).is_err());
        assert!(play_args(&["--list-audio-devices=true"]).is_err());
    }

    #[test]
    fn native_startup_honors_window_exit_and_subtitle_alias_options() {
        let parsed = play_args(&["--fullscreen", "--on-top", "--play-and-exit", "--snapshot-path=shots", "--subtitles", "captions.srt", "clip.y4m"]).unwrap();
        assert!(parsed.fullscreen && parsed.on_top && parsed.quit_at_end);
        assert_eq!(parsed.snapshot_dir, Some(PathBuf::from("shots")));
        assert_eq!(parsed.subtitle_file, Some(PathBuf::from("captions.srt")));
        let viewport = super::play_viewport(parsed.fullscreen, parsed.on_top);
        assert_eq!(viewport.fullscreen, Some(true));
        assert_eq!(viewport.window_level, Some(super::egui::WindowLevel::AlwaysOnTop));
        assert!(play_args(&["--fullscreen=false", "clip.y4m"]).is_err());
    }

    #[test]
    fn play_and_exit_waits_for_the_playlist_and_keeps_errors_visible() {
        let directory = scratch("fvid-player-exit", &[]);
        for name in ["a.y4m", "b.y4m"] { std::fs::write(directory.join(name), y4m(1)).unwrap(); }
        let mut player = Player { queue: vec![directory.join("a.y4m"), directory.join("b.y4m")], quit_at_end: true, ..Default::default() };
        player.play_index(0);
        assert!(!player.should_quit());
        player.continue_queue();
        assert_eq!(player.index, 1);
        assert!(!player.should_quit());
        player.continue_queue();
        assert!(player.should_quit());
        player.error = Some("decode error".into());
        assert!(!player.should_quit());
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn volume_walks_the_slider_and_stops_at_both_ends() {
        assert_eq!(volume_step(1_000, true), 1_050);
        assert_eq!(volume_step(1_000, false), 950);
        assert_eq!(volume_step(0, false), 0);
        assert_eq!(volume_step(VOLUME_MAX, true), VOLUME_MAX);
    }

    #[test]
    fn rate_steps_through_the_vlc_presets() {
        let mut rate = 1_000;
        let mut up = Vec::new();
        for _ in 0..4 {
            rate = rate_step(rate, true);
            up.push(rate);
        }
        assert_eq!(up, vec![1_500, 2_000, 3_000, 4_000]);
        for _ in 0..8 {
            rate = rate_step(rate, false);
        }
        assert_eq!(rate, RATES[0]);
        // A rate from the command line snaps to the next preset on the ladder.
        assert_eq!(rate_step(1_200, true), 1_500);
        assert_eq!(rate_step(1_200, false), 1_000);
    }

    #[test]
    fn the_fine_rate_walks_by_tenths_inside_the_ladder() {
        assert_eq!(rate_fine(1_000, true), 1_100);
        assert_eq!(rate_fine(1_000, false), 900);
        // Fine steps up from normal reach the top rung and stop there, the
        // same way the preset ladder does at either end: it takes thirty
        // tenths to walk 1× to 4×, and more ask for a speed that is not there.
        let mut rate = 1_000;
        for _ in 0..35 {
            rate = rate_fine(rate, true);
        }
        assert_eq!(rate, *RATES.last().unwrap());
        for _ in 0..45 {
            rate = rate_fine(rate, false);
        }
        assert_eq!(rate, RATES[0]);
        // A rate the fine pair invented is still a rung the preset keys can
        // step back from: each snaps to the neighbouring preset on its side.
        assert_eq!(rate_step(rate_fine(1_000, true), true), 1_500);
        assert_eq!(rate_step(rate_fine(1_000, false), false), 500);
    }

    #[test]
    fn a_faster_rate_shows_each_picture_for_shorter() {
        let frame = Duration::from_millis(40);
        assert_eq!(paced_period(frame, 1_000), frame);
        assert_eq!(paced_period(frame, 2_000), Duration::from_millis(20));
        assert_eq!(paced_period(frame, 500), Duration::from_millis(80));
    }

    #[test]
    fn late_repaints_do_not_slow_the_media_clock() {
        let start = Instant::now();
        let period = Duration::from_nanos(16_666_667);
        let mut deadline = start;
        // 60 fps media shown by a window repainting at 30 Hz: the clock
        // must stay behind until expired frames are discarded, not drift.
        for n in 1..=60 {
            deadline = next_frame_deadline(deadline, period, start + period * (n * 2), false);
        }
        assert_eq!(deadline, start + period * 60);
        let now = start + Duration::from_secs(10);
        assert_eq!(next_frame_deadline(deadline, period, now, true), now);
    }

    #[test]
    fn jump_modifiers_take_the_longer_rungs() {
        assert_eq!(jump_size(false, false), Duration::from_secs(3));
        assert_eq!(jump_size(true, false), Duration::from_secs(10));
        assert_eq!(jump_size(false, true), Duration::from_secs(60));
    }

    #[test]
    fn number_keys_are_tenths_of_the_timeline() {
        assert_eq!(position_from_digit(0), Some(0.0));
        assert_eq!(position_from_digit(5), Some(0.5));
        assert_eq!(position_from_digit(9), Some(0.9));
        assert_eq!(position_from_digit(10), None);
    }

    /// A wide pixel stretches the picture sideways and a tall one squeezes it; the
    /// height is the number that stands, since a pixel changes shape rather than
    /// adding rows.
    #[test]
    fn a_pixel_shape_changes_the_width_it_draws_at() {
        let coded = Vec2::new(64.0, 64.0);
        assert_eq!(display_size(coded, (1, 1)), coded);
        assert_eq!(display_size(coded, (2, 1)), Vec2::new(128.0, 64.0));
        assert_eq!(display_size(coded, (1, 2)), Vec2::new(32.0, 64.0));
        assert_eq!(display_size(coded, (4, 3)), Vec2::new(85.333336, 64.0));
    }

    /// A 16:9 picture in a 4:3 stage has the stage's width to live in, so it
    /// fits at half its coded size with a bar over and under. Zooming doubles
    /// what the fitting left, which sends the picture past the stage on every
    /// side — the part outside it is simply not shown, the way VLC's window
    /// keeps its own size under a magnification.
    #[test]
    fn a_magnified_picture_grows_past_the_frame_it_was_fitted_to() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let picture = Vec2::new(1600.0, 900.0);
        assert_eq!(
            video_rect(stage, picture, 1_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(0.0, 75.0), Pos2::new(800.0, 525.0))
        );
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(-400.0, -150.0), Pos2::new(1200.0, 750.0))
        );
        assert_eq!(
            video_rect(stage, picture, 500, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(200.0, 187.5), Pos2::new(600.0, 412.5))
        );
    }

    /// Sliding a picture only counts once it overhangs: the drag can bring an
    /// overhanging edge back over the stage, and no further, so a dragged
    /// picture never uncovers the bar the stage would have shown behind it.
    /// One that fits inside the stage cannot be moved at all.
    #[test]
    fn only_a_magnified_picture_slides_and_never_bare_an_edge() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let picture = Vec2::new(1600.0, 900.0);
        // Doubled, the picture overhangs by half a stage each way.
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::new(400.0, 150.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1600.0, 900.0))
        );
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::new(5000.0, -5000.0)),
            Rect::from_min_max(Pos2::new(0.0, -300.0), Pos2::new(1600.0, 600.0))
        );
        // A quarter of the stage has room to spare on all sides, so it stays put.
        assert_eq!(
            video_rect(stage, picture, 500, Vec2::new(900.0, 900.0)),
            video_rect(stage, picture, 500, Vec2::ZERO)
        );
    }

    #[test]
    fn control_lines_read_like_vlc() {
        assert_eq!(volume_osd(1_050, false), "Volume 105%");
        assert_eq!(volume_osd(1_050, true), "Muted");
        assert_eq!(rate_osd(1_000), "1\u{d7}");
        assert_eq!(rate_osd(1_500), "1.5\u{d7}");
        assert_eq!(rate_osd(250), "0.25\u{d7}");
        assert_eq!(zoom_osd(1_000), "Zoom 1\u{d7}");
        assert_eq!(zoom_osd(250), "Zoom 0.25\u{d7}");
        assert_eq!(crop_osd((4, 3)), "Crop 4:3");
        assert_eq!(crop_osd((235, 100)), "Crop 2.35:1");
        assert_eq!(crop_osd(NO_CROP), "Crop Default");
        assert_eq!(aspect_osd(Aspect::Shape(4, 3)), "Aspect 4:3");
        assert_eq!(aspect_osd(Aspect::Shape(235, 100)), "Aspect 2.35:1");
        assert_eq!(aspect_osd(Aspect::Source), "Aspect Default");
        assert_eq!(aspect_osd(Aspect::Fill), "Aspect Fill");
    }

    /// The four rungs are VLC's Zoom menu, and the key stops at either end
    /// rather than wrapping back to the quarter size.
    #[test]
    fn the_zoom_key_walks_vlcs_menu_and_stops_at_both_ends() {
        assert_eq!(zoom_step(1_000, true), 2_000);
        assert_eq!(zoom_step(2_000, true), 2_000);
        assert_eq!(zoom_step(250, false), 250);
        // A size from the command line snaps to the next rung of the menu.
        assert_eq!(zoom_step(1_200, true), 2_000);
        assert_eq!(zoom_step(1_200, false), 1_000);
        let mut player = Player::default();
        assert_eq!(player.zoom_milli, 1_000);
        player.apply(Control::Zoom(true));
        assert_eq!(player.zoom_milli, 2_000);
        assert_eq!(player.osd.as_ref().unwrap().0, "Zoom 2\u{d7}");
        for _ in 0..4 {
            player.apply(Control::Zoom(false));
        }
        assert_eq!(player.zoom_milli, 250);
    }

    /// `--zoom` is VLC's own option: a multiple of the fitted picture, held to
    /// the span its menu and its scale keys cover between them.
    #[test]
    fn the_command_line_hands_the_picture_a_magnification_to_start_at() {
        let parsed = play_args(&["--zoom", "2", "clip.mp4"]).unwrap();
        assert_eq!(parsed.zoom_milli, 2_000);
        assert_eq!(play_args(&["--zoom=0.5", "c"]).unwrap().zoom_milli, 500);
        assert_eq!(
            play_args(&["--zoom", "1.25", "c"]).unwrap().zoom_milli,
            1_250
        );
        assert_eq!(play_args(&["c"]).unwrap().zoom_milli, 1_000);
        for words in [
            vec!["--zoom", "0.2"],
            vec!["--zoom", "11"],
            vec!["--zoom", "wide"],
            vec!["--zoom"],
            vec!["--zoom=0"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a magnification"
            );
        }
    }

    /// VLC's list starts with the whole picture, so walking it forward far
    /// enough returns there, and the same key walked back reaches the other end
    /// of the list the same way. A shape only `--crop` named is not on the list,
    /// so the walk takes it to the first shape the list does hold.
    #[test]
    fn the_crop_key_walks_vlcs_shapes_and_back_to_the_whole_picture() {
        assert_eq!(crop_step(NO_CROP, true), (16, 10));
        assert_eq!(crop_step(CROPS[CROPS.len() - 1], true), NO_CROP);
        assert_eq!(crop_step(NO_CROP, false), (1, 1));
        assert_eq!(crop_step((3, 2), true), (16, 10));
        let mut shape = NO_CROP;
        for _ in 0..CROPS.len() {
            shape = crop_step(shape, true);
        }
        assert_eq!(shape, NO_CROP);
        let mut player = Player::default();
        assert_eq!(player.crop, NO_CROP);
        player.apply(Control::Crop(true));
        assert_eq!(player.crop, (16, 10));
        assert_eq!(player.osd.as_ref().unwrap().0, "Crop 16:10");
    }

    /// A wider picture than the shape loses its sides, a taller one its top and
    /// bottom, and what is left is as large as it can be while staying centred.
    /// The shape the file already has cuts nothing at all.
    #[test]
    fn a_crop_keeps_the_largest_centred_part_of_the_picture() {
        let square = Vec2::new(800.0, 600.0);
        assert_eq!(crop_insets(square, (1, 1), (1, 1)), [100, 0, 100, 0]);
        assert_eq!(crop_insets(square, (1, 1), (16, 9)), [0, 75, 0, 75]);
        assert_eq!(crop_insets(square, (1, 1), (16, 10)), [0, 50, 0, 50]);
        assert_eq!(crop_insets(square, (1, 1), (4, 3)), [0, 0, 0, 0]);
        // The kept rows are 335 of 600, so the row no half-step can split falls
        // to the bottom edge.
        assert_eq!(crop_insets(square, (1, 1), (239, 100)), [0, 132, 0, 133]);
        // Cropping the 16:9 file to 4:3 gives up its sides, not its height.
        assert_eq!(
            crop_insets(Vec2::new(1600.0, 900.0), (1, 1), (4, 3)),
            [200, 0, 200, 0]
        );
    }

    /// A container that stretches each pixel sideways asks for fewer of them to
    /// span a shape, so the ratio names what is seen rather than what is stored:
    /// twice-as-wide pixels make this file's 800x600 look 1600x600, and its
    /// square is 300 stored pixels across.
    #[test]
    fn a_crop_counts_the_shape_the_container_draws_and_not_the_one_stored() {
        let stored = Vec2::new(800.0, 600.0);
        let insets = crop_insets(stored, (2, 1), (1, 1));
        assert_eq!(insets, [250, 0, 250, 0]);
        assert_eq!(
            display_size(cropped_size(stored, insets), (2, 1)),
            Vec2::new(600.0, 600.0)
        );
    }

    /// The container's borders and the Crop menu's shape add up from the coded
    /// edges inward: the menu measures its square against the picture that is
    /// left after the file's own borders, and the two cuts are then one window.
    /// A file with no borders is the menu alone, and the menu's Default leaves
    /// the file's borders exactly as they are.
    #[test]
    fn the_container_borders_and_the_crop_menu_stack_from_the_coded_edges() {
        // Nothing stated by the file: the menu decides on its own, both ways.
        let plain = Vec2::new(1600.0, 900.0);
        assert_eq!(shown_insets(plain, [0; 4], (1, 1), (16, 9)), [0; 4]);
        assert_eq!(
            shown_insets(plain, [0; 4], (1, 1), (4, 3)),
            crop_insets(plain, (1, 1), (4, 3))
        );
        // 16x16 coded, four pixels cut from each side and two from top and
        // bottom leaves 8x12; the menu's square keeps 8x8 of that, so its own
        // cut is two rows at each of the remaining ends.
        let small = Vec2::new(16.0, 16.0);
        assert_eq!(
            shown_insets(small, [4, 2, 4, 2], (1, 1), (1, 1)),
            [4, 4, 4, 4]
        );
        assert_eq!(
            shown_insets(small, [4, 2, 4, 2], (1, 1), NO_CROP),
            [4, 2, 4, 2]
        );
        // Borders that cannot fit inside the frame at all — as after a failed
        // rewind to a smaller stream — are dropped rather than shown as
        // nothing, and the menu then works on the whole picture.
        assert_eq!(
            shown_insets(plain, [800, 0, 800, 0], (1, 1), (4, 3)),
            [200, 0, 200, 0]
        );
        assert_eq!(
            shown_insets(plain, [800, 0, 800, 0], (1, 1), NO_CROP),
            [0; 4]
        );
    }

    /// Ctrl+E, the accelerator VLC gives its Tools ▸ Effects and Filters menu
    /// entry, opens and closes the dialog here. The settings outlive both the
    /// closed dialog and a change of item, as the filter's values outlive them
    /// in VLC until its switch turns the filter off.
    #[test]
    fn the_effects_dialog_opens_on_vlcs_own_accelerator_and_keeps_its_values() {
        let mut player = Player::default();
        assert!(!player.effects);
        assert_eq!(player.adjust, Adjust::default());
        player.apply(Control::Effects);
        assert!(player.effects);
        player.adjust.brightness = 1.4;
        player.apply(Control::Effects);
        assert!(!player.effects);
        player.apply(Control::Effects);
        assert_eq!(player.adjust.brightness, 1.4);
    }

    /// VLC's dialog opens on its filter's defaults — every multiplier at one,
    /// the turn at nothing — and those defaults, and the switch off, are each
    /// exactly the bundle that leaves a sample alone.
    #[test]
    fn the_settings_bundle_is_the_identity_at_vlcs_defaults() {
        assert_eq!(adjust_scalars(&Adjust::default()), ADJUST_IDENTITY);
        let on = Adjust {
            on: true,
            ..Adjust::default()
        };
        assert_eq!(adjust_scalars(&on), ADJUST_IDENTITY);
    }

    /// The sliders fold into the five numbers the way VLC's adjust filter
    /// folds them: contrast scales the stored sample, brightness offsets it
    /// with the contrast's own midpoint (`(b−1)·255 + 128·(1−c)`, so raising
    /// one without the other still anchors black and white as the filter's
    /// tables do), gamma enters inverted, and the hue's turn and the
    /// saturation's scale multiply into one pair.
    #[test]
    fn the_sliders_fold_into_the_bundle_the_way_vlcs_filter_folds_them() {
        let s = adjust_scalars(&Adjust {
            on: true,
            brightness: 1.5,
            contrast: 0.5,
            gamma: 2.0,
            saturation: 3.0,
            hue: 90.0,
        });
        assert_eq!(s[0], 0.5);
        assert_eq!(s[1], 0.5 * 255.0 + 128.0 * 0.5);
        assert!((s[2] - 0.5).abs() < 1e-6);
        // A quarter turn at three times the swing: the cosine vanishes and
        // the sine carries the whole saturation.
        assert!(s[3].abs() < 1e-5);
        assert!((s[4] - 3.0).abs() < 1e-5);
    }

    /// The luma half of the bundle, on stored samples: black and white hold
    /// their anchors through contrast, a raised gamma lifts the mid-grey the
    /// way VLC's gamma slider lifts it, and the identity bundle leaves every
    /// sample at itself.
    #[test]
    fn the_adjusted_luma_holds_its_anchors_and_bends_its_midpoint() {
        let mid = adjust_luma(128.0, &ADJUST_IDENTITY);
        assert!((mid - 128.0).abs() < 1e-4);
        assert_eq!(adjust_luma(0.0, &ADJUST_IDENTITY), 0.0);
        assert!((adjust_luma(255.0, &ADJUST_IDENTITY) - 255.0).abs() < 1e-4);
        let contrast = [2.0, 128.0 * (1.0 - 2.0), 1.0, 1.0, 0.0];
        assert_eq!(adjust_luma(0.0, &contrast), 0.0);
        assert_eq!(adjust_luma(255.0, &contrast), 255.0);
        assert!((adjust_luma(192.0, &contrast) - 255.0).abs() < 1e-4);
        let gamma = [1.0, 0.0, 0.5, 1.0, 0.0];
        assert!(adjust_luma(128.0, &gamma) > 180.0);
    }

    /// The software path's pixels are already RGB, so the filter's maths runs
    /// on them through the same 601 matrix the converter used to make them.
    /// At the identity the round trip costs at most a rounding step; zeroing
    /// the saturation drains the colour out to grey at the luma the file
    /// carried; a half-turn of hue turns red into its opposite on the colour
    /// wheel, cyan, exactly where VLC's Tone slider lands it.
    #[test]
    fn the_rgb_pixels_follow_the_bundle_the_way_the_planes_do() {
        let red = [255, 40, 40, 120, 120, 120];
        let mut out = Vec::new();
        adjust_rgb(&red, &mut out, &ADJUST_IDENTITY);
        for (got, put) in out.iter().zip(&red) {
            assert!((i16::from(*got) - i16::from(*put)).abs() <= 1);
        }
        // Saturation zero: the two chroma multipliers fall to nothing.
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        // A grey, whatever the bundle, stays a grey: it has no chroma to turn.
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, -1.0, 0.4]);
        assert!(out[3] < 200);
        assert!((out[3] as i32 - out[5] as i32).abs() < 8);
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, -1.0, 0.0]);
        // Half a turn on red: green and blue rise to where red falls.
        assert!(out[0] < out[1] && out[0] < out[2]);
        assert!((out[3] as i32 - out[5] as i32).abs() < 16);
    }

    /// The region a crop keeps is the whole of the picture as far as the frame
    /// is concerned: the fitting measures what is left, so a 16:9 file cropped to
    /// 4:3 fills a 4:3 stage edge to edge instead of shrinking inside it. The
    /// texture coordinates say where the same region sits in the frame that is
    /// still uploaded whole.
    #[test]
    fn a_cropped_picture_is_fitted_to_the_frame_by_what_is_left() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let stored = Vec2::new(1600.0, 900.0);
        let insets = crop_insets(stored, (1, 1), (4, 3));
        let kept = display_size(cropped_size(stored, insets), (1, 1));
        assert_eq!(kept, Vec2::new(1200.0, 900.0));
        assert_eq!(
            video_rect(stage, kept, 1_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))
        );
        assert_eq!(uv_window(stored, insets), [0.125, 0.0, 0.875, 1.0]);
        // The menu's Default leaves both the fitting and the coordinates alone.
        let none = crop_insets(stored, (1, 1), NO_CROP);
        assert_eq!(uv_window(stored, none), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(cropped_size(stored, none), stored);
        // A square crop of a 4:3 frame spans the middle of its columns.
        assert_eq!(
            uv_window(
                Vec2::new(800.0, 600.0),
                crop_insets(Vec2::new(800.0, 600.0), (1, 1), (1, 1))
            ),
            [0.125, 0.0, 0.875, 1.0]
        );
    }

    /// A cinema ratio is the menu's own number; the label VLC gives it is what
    /// the OSD says, and a shape it never lists keeps the numbers it was given.
    #[test]
    fn crop_shapes_read_the_way_vlcs_menu_writes_them() {
        assert_eq!(crop_label(NO_CROP), "Default");
        assert_eq!(crop_label((16, 9)), "16:9");
        assert_eq!(crop_label((185, 100)), "1.85:1");
        assert_eq!(crop_label((221, 100)), "2.21:1");
        assert_eq!(crop_label((3, 2)), "3:2");
    }

    /// `--crop` is VLC's option of the same name: a shape split by `:`, which
    /// `none` clears. Everything else about it is a mistake at the door.
    #[test]
    fn the_command_line_names_the_shape_to_start_cropped_to() {
        assert_eq!(play_args(&["--crop", "16:9", "c"]).unwrap().crop, (16, 9));
        assert_eq!(play_args(&["--crop=4:3", "c"]).unwrap().crop, (4, 3));
        assert_eq!(
            play_args(&["--crop", "185:100", "c"]).unwrap().crop,
            (185, 100)
        );
        assert_eq!(play_args(&["--crop", "none", "c"]).unwrap().crop, NO_CROP);
        assert_eq!(play_args(&["c"]).unwrap().crop, NO_CROP);
        for words in [
            vec!["--crop", "16"],
            vec!["--crop", "0:1"],
            vec!["--crop", "1001:100"],
            vec!["--crop", "16:"],
            vec!["--crop", "wide"],
            vec!["--crop"],
            vec!["--crop=2:3:4"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a shape"
            );
        }
    }

    /// The five picture settings come over the command line under the names
    /// VLC gives its adjust options, minus its prefix. As in VLC, asking for
    /// one is asking for the filter: the switch goes on and the settings not
    /// named start at their defaults. A number outside its slider's span is a
    /// mistake at the door.
    #[test]
    fn the_command_line_names_the_picture_settings_to_start_with() {
        let parsed = play_args(&["--brightness", "1.5", "--hue=-30", "c"]).unwrap();
        assert!(parsed.adjust.on);
        assert_eq!(parsed.adjust.brightness, 1.5);
        assert_eq!(parsed.adjust.hue, -30.0);
        assert_eq!(parsed.adjust.contrast, 1.0);
        let parsed = play_args(&["c"]).unwrap();
        assert!(!parsed.adjust.on);
        assert_eq!(parsed.adjust, Adjust::default());
        for words in [
            vec!["--brightness", "2.5"],
            vec!["--brightness", "wide"],
            vec!["--gamma", "0"],
            vec!["--saturation", "3.1"],
            vec!["--contrast", "-1"],
            vec!["--hue", "181"],
            vec!["--hue"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a setting"
            );
        }
    }

    /// VLC's aspect-ratio menu starts at the shape the file declares, so walking
    /// it far enough returns there, and its last rung is the window's own shape.
    /// A shape only `--aspect` named is not on the menu, so the walk takes it to
    /// the first shape the menu does hold.
    #[test]
    fn the_aspect_key_walks_vlcs_shapes_and_back_to_the_files_own() {
        assert_eq!(aspect_step(Aspect::Source, true), Aspect::Shape(16, 9));
        assert_eq!(aspect_step(Aspect::Shape(5, 4), true), Aspect::Fill);
        assert_eq!(
            aspect_step(ASPECTS[ASPECTS.len() - 1], true),
            Aspect::Source
        );
        assert_eq!(aspect_step(Aspect::Source, false), Aspect::Fill);
        assert_eq!(aspect_step(Aspect::Shape(5, 3), true), Aspect::Shape(16, 9));
        let mut shape = Aspect::Source;
        for _ in 0..ASPECTS.len() {
            shape = aspect_step(shape, true);
        }
        assert_eq!(shape, Aspect::Source);
        let mut player = Player::default();
        assert_eq!(player.aspect, Aspect::Source);
        player.apply(Control::Aspect(true));
        assert_eq!(player.aspect, Aspect::Shape(16, 9));
        assert_eq!(player.osd.as_ref().unwrap().0, "Aspect 16:9");
    }

    /// A forced shape is the proportion the picture is drawn at, whatever the
    /// container said: the height stands and the width follows, as it does for
    /// the file's own pixels. `Fill` asks for the frame instead of a number.
    #[test]
    fn a_forced_shape_is_the_proportion_the_picture_is_drawn_at() {
        let square = Vec2::new(64.0, 64.0);
        assert_eq!(
            shown_size(square, (2, 1), Aspect::Source, Vec2::new(1280.0, 720.0)),
            Vec2::new(128.0, 64.0)
        );
        assert_eq!(
            shown_size(
                Vec2::new(800.0, 600.0),
                (2, 1),
                Aspect::Shape(4, 3),
                Vec2::ZERO
            ),
            Vec2::new(800.0, 600.0)
        );
        assert_eq!(
            shown_size(
                Vec2::new(1920.0, 1080.0),
                (1, 1),
                Aspect::Shape(16, 9),
                Vec2::ZERO
            ),
            Vec2::new(1920.0, 1080.0)
        );
        assert_eq!(
            shown_size(square, (1, 1), Aspect::Shape(235, 100), Vec2::ZERO),
            Vec2::new(150.4, 64.0)
        );
        assert_eq!(
            shown_size(square, (1, 1), Aspect::Fill, Vec2::new(1280.0, 720.0)),
            Vec2::new(1280.0, 720.0)
        );
    }

    /// The forced shape reaches the screen through the same fitting as the
    /// container's own: a square file asked to be twice as wide as tall spans the
    /// stage's width and is letterboxed above and below, and `Fill` leaves
    /// nothing to letterbox.
    #[test]
    fn a_forced_shape_is_fitted_to_the_frame_like_the_files_own() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let square = Vec2::new(64.0, 64.0);
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Shape(2, 1), stage.size()),
                1_000,
                Vec2::ZERO
            ),
            Rect::from_min_max(Pos2::new(0.0, 100.0), Pos2::new(800.0, 500.0))
        );
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Fill, stage.size()),
                1_000,
                Vec2::ZERO
            ),
            stage
        );
        // Leaving the shape alone keeps the square it always drew.
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Source, stage.size()),
                1_000,
                Vec2::ZERO
            ),
            Rect::from_min_max(Pos2::new(100.0, 0.0), Pos2::new(700.0, 600.0))
        );
    }

    /// A crop is cut from the stored picture before a shape is forced on what is
    /// left, the way VLC's two overrides stack, so a 4:3 crop of a 16:9 file
    /// under a forced 16:9 ends up 16:9 again. The crop still measures itself
    /// against the shape the container declared, forced shape or not.
    #[test]
    fn a_crop_and_a_forced_shape_apply_one_after_the_other() {
        let stored = Vec2::new(1600.0, 900.0);
        let insets = crop_insets(stored, (1, 1), (4, 3));
        assert_eq!(
            shown_size(
                cropped_size(stored, insets),
                (1, 1),
                Aspect::Shape(16, 9),
                Vec2::ZERO
            ),
            Vec2::new(1600.0, 900.0)
        );
        let wide = crop_insets(Vec2::new(800.0, 600.0), (2, 1), (1, 1));
        assert_eq!(wide, [250, 0, 250, 0]);
        assert_eq!(
            uv_window(Vec2::new(800.0, 600.0), wide),
            [0.3125, 0.0, 0.6875, 1.0]
        );
    }

    /// A cinema ratio is the menu's own number and its label is what the OSD
    /// says; the two rungs that name no shape say so in words.
    #[test]
    fn aspect_shapes_read_the_way_vlcs_menu_writes_them() {
        assert_eq!(aspect_label(Aspect::Source), "Default");
        assert_eq!(aspect_label(Aspect::Shape(16, 9)), "16:9");
        assert_eq!(aspect_label(Aspect::Shape(235, 100)), "2.35:1");
        assert_eq!(aspect_label(Aspect::Shape(221, 100)), "2.21:1");
        assert_eq!(aspect_label(Aspect::Shape(3, 2)), "3:2");
        assert_eq!(aspect_label(Aspect::Fill), "Fill");
    }

    /// `--aspect` is VLC's option of the same name: a shape split by `:`, or one
    /// of the two words for the rungs that name no shape. Everything else about
    /// it is a mistake at the door.
    #[test]
    fn the_command_line_names_the_shape_to_start_drawn_at() {
        assert_eq!(
            play_args(&["--aspect", "16:9", "c"]).unwrap().aspect,
            Aspect::Shape(16, 9)
        );
        assert_eq!(
            play_args(&["--aspect=4:3", "c"]).unwrap().aspect,
            Aspect::Shape(4, 3)
        );
        assert_eq!(
            play_args(&["--aspect", "235:100", "c"]).unwrap().aspect,
            Aspect::Shape(235, 100)
        );
        assert_eq!(
            play_args(&["--aspect", "fill", "c"]).unwrap().aspect,
            Aspect::Fill
        );
        assert_eq!(
            play_args(&["--aspect", "default", "c"]).unwrap().aspect,
            Aspect::Source
        );
        assert_eq!(play_args(&["c"]).unwrap().aspect, Aspect::Source);
        for words in [
            vec!["--aspect", "16"],
            vec!["--aspect", "0:1"],
            vec!["--aspect", "1001:100"],
            vec!["--aspect", "16:"],
            vec!["--aspect", "wide"],
            vec!["--aspect"],
            vec!["--aspect=2:3:4"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not an aspect"
            );
        }
    }

    /// The rate is written whole whenever it comes within a twentieth of a frame
    /// of one, which is how close the NTSC pull-down is: 41.7 ms per frame reads
    /// as `24`, and only a rate standing further off keeps its decimals. A
    /// stream with no period has no rate at all.
    #[test]
    fn a_frame_rate_is_whole_when_it_rounds_to_whole() {
        assert_eq!(fps_text(Duration::from_millis(40)), "25");
        assert_eq!(fps_text(Duration::from_nanos(41_708_333)), "24");
        assert_eq!(fps_text(Duration::from_nanos(42_553_191)), "23.50");
        assert_eq!(fps_text(Duration::ZERO), "0");
    }

    /// Sizes are counted by a thousand, the way a disk's capacity is named, and
    /// keep a decimal only while the number has a single digit to give.
    #[test]
    fn a_file_size_takes_the_largest_unit_it_fills() {
        assert_eq!(byte_size(812), "812 B");
        assert_eq!(byte_size(1_000), "1.0 kB");
        assert_eq!(byte_size(9_440_000), "9.4 MB");
        assert_eq!(byte_size(12_345_678), "12 MB");
        assert_eq!(byte_size(4_500_000_000_000), "4.5 TB");
    }

    /// The rate a whole file adds up to needs both of its numbers, so a timeline
    /// the container never stated leaves the panel without one.
    #[test]
    fn a_bitrate_needs_a_size_and_a_length() {
        assert_eq!(
            bitrate_text(1_000_000, Duration::from_secs(2)).as_deref(),
            Some("4.0 Mb/s")
        );
        assert_eq!(
            bitrate_text(16_000, Duration::from_secs(1)).as_deref(),
            Some("128 kb/s")
        );
        assert_eq!(bitrate_text(16_000, Duration::ZERO), None);
    }

    /// The size is read as the item opens, from the path it came by; one the
    /// file system no longer answers for is left out of the panel.
    #[test]
    fn a_size_comes_from_the_path_and_nothing_else() {
        let directory = scratch("fvid-player-info-size", &[]);
        let path = directory.join("size.bin");
        std::fs::write(&path, b"twelve bytes").unwrap();
        assert_eq!(file_size(&path), Some(12));
        assert_eq!(file_size(&directory.join("absent.bin")), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The buffer and spool lines count progress that moves while the panel is
    /// read, so their numbers differ between runs; the panel tests hold every
    /// other line to an exact list and flatten those two to fixed words.
    fn panel_lines(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if line.starts_with("Buffer ") {
                    "Buffer …".to_owned()
                } else if line.starts_with("Spool ") {
                    "Spool …".to_owned()
                } else {
                    line.clone()
                }
            })
            .collect()
    }

    /// The panel says what the open item is in the player's own words: the
    /// picture names the codec its container tagged, the size it is stored at
    /// and the rate it runs at; a track names its place in the file's own list;
    /// the file names its size and the rate that size adds up to over its
    /// timeline.
    /// `tests/fixtures/tracks/named.mp4`: 64×64 H.264 at 25 fps over less than a
    /// second, three audio tracks and two subtitle tracks.
    #[test]
    fn the_panel_lists_what_the_open_item_carries() {
        let directory = scratch("fvid-player-info-panel", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(!player.info, "the panel starts closed");
        player.apply(Control::Info);
        // On macOS the H.264 in this fixture opens a VideoToolbox session, and
        // the panel says so; which of the two this machine gets is the flag's
        // own answer, so the panel is held to it rather than to one build.
        let decoder = if player.hardware {
            " · VideoToolbox"
        } else {
            ""
        };
        assert_eq!(
            panel_lines(&player.info_lines()),
            [
                format!("Video: H.264 · 64×64 · 25 fps{decoder}"),
                "Buffer …".to_owned(),
                "Spool …".to_owned(),
                "Sound: 1/3 · AAC · Первая · 1 ch 48000 Hz".to_owned(),
                "Subtitles: 1/2 · Титры".to_owned(),
                "File: 21 kB · 0:00 · 413 kb/s".to_owned(),
            ],
            "{:?}",
            player.info_lines()
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A Matroska item names the codec its own track types carry, and the parts
    /// its container lists for it come last.
    /// `tests/fixtures/chapters/chapters.mkv`: 16×16 VP9 at four frames a second
    /// over four seconds, no sound, three named parts.
    #[test]
    fn a_matroska_item_names_its_own_codec_and_parts() {
        let directory = scratch("fvid-player-info-matroska", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            panel_lines(&player.info_lines()),
            [
                "Video: VP9 · 16×16 · 4 fps",
                "Buffer …",
                "Spool …",
                "File: 1.2 kB · 0:04 · 2 kb/s",
                "Chapters: 3",
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The name a container gives the file as a whole leads the panel, in both
    /// families and for an item that is nothing but sound: the same walk that
    /// finds the file's parts finds its name.
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=1 \
    ///   -metadata title="Полное имя элемента" -c:v libx264 -pix_fmt yuv420p -an \
    ///   tests/fixtures/tags/title.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title="Песня без картинки" -c:a flac -vn tests/fixtures/tags/song.mkv
    /// ```
    /// The first is 16×16 H.264 at four frames a second over one second, named by
    /// its `moov/udta`; the second is one second of FLAC with no picture at all,
    /// named the same way. `tests/fixtures/tags/title.mkv` — the same picture in
    /// VP9, named by its information block — is the file `container::webm`
    /// documents and reads.
    #[test]
    fn the_panel_leads_with_the_name_the_container_gives_the_file() {
        let directory = scratch("fvid-player-info-name", &[]);
        let panel = |name: &str, bytes: &[u8]| -> Vec<String> {
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            drop(player);
            lines
        };
        for (name, bytes, title) in [
            (
                "title.mp4",
                &include_bytes!("../../../tests/fixtures/tags/title.mp4")[..],
                "Полное имя элемента",
            ),
            (
                "title.mkv",
                &include_bytes!("../../../tests/fixtures/tags/title.mkv")[..],
                "Имя из контейнера",
            ),
            (
                "song.mkv",
                &include_bytes!("../../../tests/fixtures/tags/song.mkv")[..],
                "Песня без картинки",
            ),
        ] {
            assert_eq!(panel(name, bytes)[0], format!("Name: {title}"), "{name}");
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The rest of what a container states about the file as a whole stands in
    /// the panel beside its name, in the order the dialog these facts come by
    /// lists them — in both families, whose writers spell the same six facts
    /// their own way.
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B \
    ///   -metadata genre=G -metadata date=2026 -metadata comment=C \
    ///   -c:v libx264 -pix_fmt yuv420p -an tests/fixtures/tags/tags.mp4
    /// ```
    /// `tests/fixtures/tags/tags.mkv` carries the same six facts on the same
    /// picture in VP9, by the command `container::webm` documents.
    #[test]
    fn the_panel_keeps_every_fact_the_container_states_about_the_file() {
        let directory = scratch("fvid-player-info-tags", &[]);
        for (name, bytes) in [
            (
                "tags.mp4",
                &include_bytes!("../../../tests/fixtures/tags/tags.mp4")[..],
            ),
            (
                "tags.mkv",
                &include_bytes!("../../../tests/fixtures/tags/tags.mkv")[..],
            ),
        ] {
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert_eq!(
                lines[..6],
                [
                    "Name: T",
                    "Artist: A",
                    "Album: B",
                    "Genre: G",
                    "Date: 2026",
                    "Comment: C",
                ],
                "{name}"
            );
            drop(player);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file whose container names nothing has no line to lead with: the panel
    /// starts at its picture, and the name the file carries where it lies stays
    /// where the player has always shown it, above the panel.
    /// `tests/fixtures/chapters/chapters.mkv` states no title at all.
    #[test]
    fn an_unnamed_item_opens_its_panel_with_its_picture() {
        let directory = scratch("fvid-player-info-unnamed", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.file_tags, FileTags::default());
        assert_eq!(player.info_lines()[0], "Video: VP9 · 16×16 · 4 fps");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file muxed to be drawn twice as wide as it is stored says so on its own
    /// line: the shape comes from the container and follows the stored size the
    /// pixels are measured in.
    /// `tests/fixtures/display/par-2x1.webm`: 64×64 VP9 at 25 fps, pixels 2:1.
    #[test]
    fn an_anamorphic_item_names_the_shape_of_its_pixels() {
        let directory = scratch("fvid-player-info-anamorphic", &[]);
        let path = directory.join("par-2x1.webm");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/display/par-2x1.webm"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            player.info_lines()[0],
            "Video: VP9 · 64×64 · 25 fps · pixels 2:1"
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The panel is not a message that fades away: it holds the corner it is
    /// drawn in open for as long as it is up, the way a paused item does, and the
    /// same key that opened it closes it.
    #[test]
    fn the_panel_stays_until_the_key_closes_it() {
        let directory = scratch("fvid-player-info-keeps", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        player.apply(Control::Info);
        player.activity = Instant::now() - HIDE_AFTER;
        assert!(player.controls_visible(), "an open panel holds the corner");
        player.apply(Control::Info);
        assert!(!player.info);
        assert!(!player.controls_visible(), "a closed panel lets it fade");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An item with nothing to show has no picture to name, so the panel starts
    /// at the sound it is playing and stops at the file around it.
    /// `tests/fixtures/audio/flac-stereo.mkv`: two channels at 44.1 kHz over two
    /// seconds, and no video track at all.
    #[test]
    fn a_sound_only_item_is_described_by_its_track() {
        let directory = scratch("fvid-player-info-sound-only", &[]);
        let path = directory.join("flac-stereo.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.video_codec, "");
        let lines = panel_lines(&player.info_lines());
        assert_eq!(
            lines,
            [
                "Spool …",
                "Sound: 1/1 · FLAC · 2 ch 44100 Hz",
                // The length comes from the item's own two seconds, which is the
                // one statement the panel has before every block is indexed;
                // the rate it gives is the one ffmpeg prints for this file.
                "File: 105 kB · 0:02 · 419 kb/s",
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One coding, however many containers spell it: the table gives the name a
    /// viewer expects for every tag this build has a decoder for, and a coding the
    /// table does not know keeps the name its own file gave it.
    #[test]
    fn every_sound_this_build_decodes_has_the_name_a_viewer_gives_it() {
        for (tag, named) in [
            ("A_OGG/VORBIS_OPUS", "Vorbis/Opus"),
            ("mp4a", "AAC"),
            ("A_VORBIS", "Vorbis"),
            ("A_FLAC", "FLAC"),
            ("A_MPEG/L3", "MP3"),
            ("A_MPEG/L2", "MP2"),
            ("A_ALAC", "ALAC"),
            ("alac", "ALAC"),
            ("A_AC3", "Dolby Digital"),
            ("ac-3", "Dolby Digital"),
            ("A_PCM/INT/LIT", "PCM"),
            ("A_PCM/INT/BIG", "PCM"),
            ("A_PCM/FLOAT/IEEE", "PCM"),
            ("sowt", "PCM"),
            ("twos", "PCM"),
            ("fl32", "PCM"),
            ("fl64", "PCM"),
            ("pcm_alaw", "G.711 (a-law)"),
            ("pcm_mulaw", "G.711 (mu-law)"),
            ("adpcm_ms", "ADPCM (MS)"),
            ("adpcm_ima_wav", "ADPCM (IMA)"),
            ("adpcm_ima_qt", "ADPCM (IMA)"),
            ("midi", "MIDI"),
            ("xm", "XM"),
            (
                "A_THE_PLACE_NO_READER_COMES_FROM",
                "A_THE_PLACE_NO_READER_COMES_FROM",
            ),
        ] {
            assert_eq!(sound_codec(tag), named, "{tag}");
        }
    }

    /// The panel names the coding the sound on screen is actually reading, and
    /// the track key changes that name together with the sound.
    /// `tests/fixtures/audio/ac3-flac-mp3.mkv`: an AC-3 track, then FLAC and MP3.
    /// All three are in the list now that the reader hands `A_AC3` to the decoder,
    /// so the key walks the panel from Dolby Digital through the two lossy ones and
    /// the count it prints is the three the file offers.
    #[test]
    fn the_panel_names_the_codec_and_the_track_key_keeps_it_current() {
        let directory = scratch("fvid-player-sound-codec", &[]);
        let path = directory.join("ac3-flac-mp3.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/ac3-flac-mp3.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        let sound = |player: &Player| {
            player
                .info_lines()
                .into_iter()
                .find(|line| line.starts_with("Sound: "))
                .unwrap_or_default()
        };
        assert!(
            sound(&player).starts_with("Sound: 1/3 · Dolby Digital · "),
            "{}",
            sound(&player)
        );
        player.cycle_audio_track(true);
        assert_eq!(
            (player.audio_track, player.sound_codec.as_str()),
            (1, "FLAC")
        );
        player.cycle_audio_track(true);
        assert_eq!(
            (player.audio_track, player.sound_codec.as_str()),
            (2, "MP3")
        );
        assert!(
            sound(&player).starts_with("Sound: 3/3 · MP3 · "),
            "{}",
            sound(&player)
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One fact, two writings: ffmpeg puts `track=3/12` into MP4 as a binary
    /// `trkn` pair and into Matroska as the text `3/12` under `PART_NUMBER`,
    /// and the panel answers both the same way, with the track standing right
    /// after the album it is a place in.
    /// ```text
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B -metadata track=3/12 \
    ///   -c:a aac tests/fixtures/tags/track.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B -metadata track=3/12 \
    ///   -c:a flac tests/fixtures/tags/track.mkv
    /// ```
    #[test]
    fn the_panel_lists_the_place_a_file_names_in_its_album() {
        for (name, bytes) in [
            (
                "track.mp4",
                &include_bytes!("../../../tests/fixtures/tags/track.mp4")[..],
            ),
            (
                "track.mkv",
                &include_bytes!("../../../tests/fixtures/tags/track.mkv")[..],
            ),
        ] {
            let directory = scratch("fvid-player-info-track", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert_eq!(
                lines[..4],
                ["Name: T", "Artist: A", "Album: B", "Track: 3/12",],
                "{name}"
            );
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// One round of ffmpeg's `-metadata` with the album's own facts, the two
    /// containers parting ways exactly as their formats do:
    /// ```text
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata album_artist=AA -metadata disc=2/10 \
    ///   -metadata publisher=PB -c:a aac tests/fixtures/tags/band.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata album_artist=AA -metadata disc=2/10 \
    ///   -metadata publisher=PB -c:a flac tests/fixtures/tags/band.mkv
    /// ```
    /// MP4 has no box for a publisher and the muxer drops that fact on the
    /// floor; Matroska keeps every word it was handed. The disc's number-pair
    /// arrives in both, the binary writing in one file and the text in the
    /// other, and the panel places each fact between the lines it belongs to.
    #[test]
    fn the_panel_names_the_album_its_artist_its_disc_and_its_publisher() {
        for (name, bytes, facts) in [
            (
                "band.mp4",
                &include_bytes!("../../../tests/fixtures/tags/band.mp4")[..],
                ["Name: T", "Album artist: AA", "Disc: 2/10"].as_slice(),
            ),
            (
                "band.mkv",
                &include_bytes!("../../../tests/fixtures/tags/band.mkv")[..],
                ["Name: T", "Album artist: AA", "Disc: 2/10", "Publisher: PB"].as_slice(),
            ),
        ] {
            let directory = scratch("fvid-player-info-album", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            for fact in facts {
                assert!(lines.contains(&fact.to_string()), "{name}: {lines:?}");
            }
            if name.ends_with(".mp4") {
                assert!(
                    !lines.iter().any(|l| l.starts_with("Publisher")),
                    "a fact the writer dropped cannot be shown: {lines:?}"
                );
            }
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// Two facts one writer states and the other forgets: ffmpeg answers
    /// `-metadata copyright` and `-metadata description` in both containers,
    /// filing them in MP4 as the unmarked `cprt` and `desc` atoms and in
    /// Matroska under their plain names, and the panel shows each where the
    /// file stated it.
    /// `tests/fixtures/tags/rights.mp4` and `rights.mkv`:
    /// ffmpeg -f lavfi -i anullsrc=r=8000:cl=mono -t 0.2 \
    ///   -metadata title=T -metadata copyright='2026 The Holder' \
    ///   -metadata description='A note' -c:a aac tests/fixtures/tags/rights.mp4
    /// (the same line with `-c:a flac` writes the `.mkv`), and ffprobe reads
    /// both facts back from each.
    #[test]
    fn the_panel_shows_the_rights_and_the_note_where_the_file_stated_them() {
        for (name, bytes) in [
            (
                "rights.mp4",
                &include_bytes!("../../../tests/fixtures/tags/rights.mp4")[..],
            ),
            (
                "rights.mkv",
                &include_bytes!("../../../tests/fixtures/tags/rights.mkv")[..],
            ),
        ] {
            let directory = scratch("fvid-player-info-rights", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            for fact in [
                "Name: T",
                "Copyright: 2026 The Holder",
                "Description: A note",
            ] {
                assert!(lines.contains(&fact.to_string()), "{name}: {lines:?}");
            }
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// The one fact only one of the two containers carries: ffmpeg answers
    /// `-metadata rating` in Matroska under that plain name and drops it from
    /// an MP4 without a word, and the panel shows the rating where the file
    /// kept it and not otherwise. `tests/fixtures/tags/rated.mkv` and
    /// `rated.mp4`:
    /// ffmpeg -f lavfi -i anullsrc=r=8000:cl=mono -t 0.2 \
    ///   -metadata title=T -metadata rating=5 -c:a flac tests/fixtures/tags/rated.mkv
    /// (the same line with `-c:a aac` writes the `.mp4`), and ffprobe reads the
    /// rating back from the Matroska file and finds no trace of it in the MP4.
    #[test]
    fn the_panel_shows_the_rating_the_matroska_file_was_given() {
        for (name, bytes, stated) in [
            (
                "rated.mkv",
                &include_bytes!("../../../tests/fixtures/tags/rated.mkv")[..],
                true,
            ),
            (
                "rated.mp4",
                &include_bytes!("../../../tests/fixtures/tags/rated.mp4")[..],
                false,
            ),
        ] {
            let directory = scratch("fvid-player-info-rating", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert!(lines.contains(&"Name: T".to_string()), "{name}: {lines:?}");
            assert_eq!(
                lines.contains(&"Rating: 5".to_string()),
                stated,
                "{name}: {lines:?}"
            );
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// With no item on screen there is nothing to describe, so the key says so
    /// instead of opening an empty panel over the stage.
    #[test]
    fn the_information_key_has_nothing_to_name() {
        let mut player = Player::default();
        player.apply(Control::Info);
        assert!(!player.info);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing open");
    }

    fn subtitled() -> Player {
        Player {
            cues: subtitles::parse(
                "1\n00:00:01,000 --> 00:00:02,000\n<i>Hi</i>\n\n2\n00:00:03,000 --> 00:00:04,000\nThere\n",
            ),
            ..Default::default()
        }
    }

    fn seconds(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
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

    #[test]
    fn native_sami_sync_clocks_clear_and_seek_like_independent_subrip() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/sami-sync");
        for name in ["language-gap", "invalid-clock"] {
            let expected = subtitles::load(&root.join(format!("{name}.srt"))).unwrap();
            let mut player = Player {
                queue: vec![root.parent().unwrap().join("vp9-long-gop-seek.webm")],
                subtitle_file: Some(root.join(format!("{name}.smi"))),
                no_audio: true,
                asynchronous_open: false,
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            assert_eq!(player.cues, expected, "{name}");
            let ctx = eframe::egui::Context::default();
            let until = Instant::now() + Duration::from_secs(10);
            while player.presented.is_none() {
                assert!(Instant::now() < until, "{name}: no native frame");
                let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            for millis in [500, 1500, 2500, 3500, 4500, 1500, 5000] {
                let target = Duration::from_millis(millis);
                player.seek_time(target);
                let until = Instant::now() + Duration::from_secs(10);
                while player.seek_preview || player.buffering {
                    assert!(player.error.is_none(), "{name}: {:?}", player.error);
                    assert!(Instant::now() < until, "{name}: seek {millis} stalled");
                    let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                    output.textures_delta.clear();
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(player.presented.as_ref().unwrap().generation,
                    player.playback.as_ref().unwrap().generation());
                let line = subtitles::active(&expected, target, Duration::ZERO)
                    .map(|cue| cue.text.as_str());
                assert_eq!(player.subtitle_line(target), line, "{name} at {millis}");
            }
        }
    }

    fn wait_for_native_caption_frame(player: &mut Player, ctx: &eframe::egui::Context) {
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.playback.is_some());
        let until = Instant::now() + Duration::from_secs(10);
        while player.presented.is_none() || player.seek_preview || player.buffering {
            assert!(player.error.is_none(), "{:?}", player.error);
            assert!(Instant::now() < until, "native caption frame did not arrive");
            let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn native_sami_break_case_preserves_caption_lines() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/sami-break-case");
        let mut player = Player {
            queue: vec![root.parent().unwrap().join("vp9-long-gop-seek.webm")],
            subtitle_file: Some(root.join("upper.smi")),
            no_audio: true,
            asynchronous_open: false,
            ..Default::default()
        };
        player.play_index(0);
        let ctx = eframe::egui::Context::default();
        wait_for_native_caption_frame(&mut player, &ctx);
        assert_eq!(player.subtitle_line(Duration::from_millis(1500)), Some("Первая\nстрока"));
        for (millis, line) in [(1500, Some("Первая\nстрока")), (2500, None), (500, None), (1500, Some("Первая\nстрока"))] {
            player.seek_time(Duration::from_millis(millis));
            wait_for_native_caption_frame(&mut player, &ctx);
            assert_eq!(player.subtitle_line(Duration::from_millis(millis)), line);
        }
    }

    #[test]
    fn native_sami_extension_is_discovered_and_survives_seeking() {
        let directory = scratch("fvid-native-sami-extension", &[]);
        std::fs::write(directory.join("video.webm"),
            include_bytes!("../../../tests/fixtures/playback-errors/vp9-long-gop-seek.webm")).unwrap();
        std::fs::write(directory.join("video.sami"),
            include_bytes!("../../../tests/fixtures/playback-errors/sami-sidecar-extension/lower/video.sami")).unwrap();
        let mut player = Player {
            queue: vec![directory.join("video.webm")],
            no_audio: true,
            asynchronous_open: false,
            ..Default::default()
        };
        player.play_index(0);
        let ctx = eframe::egui::Context::default();
        wait_for_native_caption_frame(&mut player, &ctx);
        assert_eq!(player.subtitle_name, "video.sami");
        for (millis, line) in [(1500, Some("Строка α")), (2500, None), (500, None), (1500, Some("Строка α"))] {
            player.seek_time(Duration::from_millis(millis));
            wait_for_native_caption_frame(&mut player, &ctx);
            assert_eq!(player.subtitle_line(Duration::from_millis(millis)), line);
        }
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_sami_body_boundaries_preserve_caption_and_seek() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/sami-body-boundaries");
        for name in ["quoted-body-close", "title-raw-close"] {
            let mut player = Player {
                queue: vec![root.parent().unwrap().join("vp9-long-gop-seek.webm")],
                subtitle_file: Some(root.join(format!("{name}.smi"))),
                no_audio: true,
                asynchronous_open: false,
                ..Default::default()
            };
            player.play_index(0);
            let ctx = eframe::egui::Context::default();
            wait_for_native_caption_frame(&mut player, &ctx);
            assert_eq!(player.subtitle_line(Duration::from_millis(1500)),
                Some("Строка α"), "{name}");
            for (millis, line) in [(500, None), (1500, Some("Строка α")), (2500, None), (1500, Some("Строка α"))] {
                let target = Duration::from_millis(millis);
                player.seek_time(target);
                wait_for_native_caption_frame(&mut player, &ctx);
                assert_eq!(player.presented.as_ref().unwrap().generation,
                    player.playback.as_ref().unwrap().generation());
                assert_eq!(player.subtitle_line(target), line, "{name} at {millis}");
            }
        }
    }

    #[test]
    fn cues_are_live_inside_their_span() {
        let player = subtitled();
        assert_eq!(player.subtitle_line(seconds(1_000)), Some("Hi"));
        assert_eq!(player.subtitle_line(seconds(1_999)), Some("Hi"));
        assert_eq!(player.subtitle_line(seconds(2_000)), None);
        assert_eq!(player.subtitle_line(seconds(3_500)), Some("There"));
    }

    /// The delay key moves the whole track, so a later cue is asked for what
    /// the file held earlier; pressing it back past zero shows nothing before
    /// the first cue rather than wrapping around.
    #[test]
    fn the_delay_key_shifts_the_cue_timeline() {
        let mut player = subtitled();
        player.apply(Control::SubtitleDelay(true));
        assert_eq!(player.subtitle_delay_ms, 50);
        assert_eq!(player.subtitle_line(seconds(1_000)), None);
        assert_eq!(player.subtitle_line(seconds(1_050)), Some("Hi"));

        player.apply(Control::SubtitleDelay(false));
        player.apply(Control::SubtitleDelay(false));
        assert_eq!(player.subtitle_delay_ms, -50);
        assert_eq!(player.subtitle_line(seconds(950)), Some("Hi"));

        player.apply(Control::SubtitleDelay(true));
        player.apply(Control::SubtitleDelay(true));
        assert_eq!(player.subtitle_delay_ms, 50);
        assert_eq!(player.subtitle_line(seconds(0)), None);
    }

    #[test]
    fn subtitles_toggle_and_survive_an_empty_track() {
        let mut player = Player::default();
        player.apply(Control::Subtitles);
        assert_eq!(player.osd.as_ref().unwrap().0, "No subtitles");
        assert!(player.subtitle_shown);

        player = subtitled();
        player.apply(Control::Subtitles);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles off");
        assert_eq!(player.subtitle_line(seconds(1_000)), None);
        player.apply(Control::Subtitles);
        assert_eq!(player.subtitle_line(seconds(1_000)), Some("Hi"));
    }

    /// The delay key is bounded at plus and minus ten seconds, so a track
    /// pushed past its end stays where it is rather than running away.
    #[test]
    fn the_delay_key_is_bounded_at_ten_seconds() {
        assert_eq!(delay_step(0, true), 50);
        assert_eq!(delay_step(0, false), -50);
        assert_eq!(delay_step(10_000, true), 10_000);
        assert_eq!(delay_step(-10_000, false), -10_000);
    }

    /// A file beside the video is found on its own, without being asked for.
    #[test]
    fn a_video_picks_up_the_sidecar_beside_it() {
        let directory = std::env::temp_dir().join("fvid-player-subtitle-sidecar");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("clip.webm"), b"not really a video").unwrap();
        std::fs::write(
            directory.join("clip.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n",
        )
        .unwrap();

        let mut player = Player::default();
        player.load_subtitles(&directory.join("clip.webm"));
        assert_eq!(player.subtitle_name, "clip.srt");
        assert_eq!(
            player.subtitle_line(Duration::from_millis(1_500)),
            Some("Hello")
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file the viewer names joins the captions the video came with and is
    /// turned to at once, since it was asked for after everything else.
    #[test]
    fn a_named_file_joins_the_captions_of_the_open_item() {
        let directory = scratch("fvid-player-subtitle-load", &[]);
        std::fs::write(
            directory.join("first.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nEarly\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("later.srt"),
            "1\n00:00:05,000 --> 00:00:06,000\nLate\n",
        )
        .unwrap();

        let mut player = Player::default();
        player.subtitle_sources.push(SubtitleSource {
            label: "first.srt".to_owned(),
            cues: subtitles::parse("1\n00:00:01,000 --> 00:00:02,000\nEarly\n"),
        });
        player.apply_subtitle_source();
        player.add_subtitle_file(directory.join("later.srt"));
        assert_eq!(
            player
                .subtitle_sources
                .iter()
                .map(|source| source.label.as_str())
                .collect::<Vec<_>>(),
            ["first.srt", "later.srt"]
        );
        assert_eq!(
            (player.subtitle_source, player.subtitle_name.as_str()),
            (1, "later.srt")
        );
        assert!(player.subtitle_shown);
        assert_eq!(player.subtitle_line(seconds(5_500)), Some("Late"));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles: later.srt");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file nothing can be read from leaves the captions as they were and
    /// says so, the way a `--sub-file` that yields nothing does.
    #[test]
    fn a_file_that_captions_nothing_leaves_the_captions_alone() {
        let mut player = subtitled();
        player.subtitle_sources.push(SubtitleSource {
            label: "the-file.srt".to_owned(),
            cues: player.cues.clone(),
        });
        player.apply_subtitle_source();
        let before = player.subtitle_name.clone();
        player.add_subtitle_file(std::env::temp_dir().join("fvid-no-such-subtitles.srt"));
        assert_eq!(player.subtitle_sources.len(), 1);
        assert_eq!(player.subtitle_name, before);
        assert_eq!(player.subtitle_line(seconds(1_500)), Some("Hi"));
        assert!(
            player
                .osd
                .as_ref()
                .unwrap()
                .0
                .starts_with("Cannot read subtitles"),
            "{:?}",
            player.osd
        );
    }

    /// An item that is not open has nothing to be captioned by, so the load
    /// key says so rather than opening a picker whose answer would be dropped.
    #[test]
    fn the_load_key_has_nothing_to_caption() {
        let mut player = Player::default();
        player.apply(Control::LoadSubtitles);
        assert!(player.dialog.is_none());
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to caption");
    }

    #[test]
    fn subtitle_size_and_margin_have_ends() {
        let mut player = subtitled();
        for _ in 0..40 {
            player.apply(Control::SubtitleSize(true));
            player.apply(Control::SubtitleMargin(true));
        }
        assert_eq!(player.subtitle_font, 72.0);
        assert_eq!(player.subtitle_margin, 8.0);
        for _ in 0..80 {
            player.apply(Control::SubtitleSize(false));
            player.apply(Control::SubtitleMargin(false));
        }
        assert_eq!(player.subtitle_font, 10.0);
        assert_eq!(player.subtitle_margin, 400.0);
    }

    /// A temporary directory holding `files`, for the list tests.
    fn scratch(name: &str, files: &[&str]) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        for file in files {
            std::fs::write(directory.join(file), b"x").unwrap();
        }
        directory
    }

    #[test]
    fn the_list_advances_until_the_end_and_then_per_repetition() {
        // Three items: forward stops past the last one unless the list repeats.
        assert_eq!(advance(0, 3, Repeat::Off), Some(1));
        assert_eq!(advance(2, 3, Repeat::Off), None);
        assert_eq!(advance(2, 3, Repeat::All), Some(0));
        assert_eq!(advance(0, 0, Repeat::All), None);
        assert_eq!(retreat(1, 3, Repeat::Off), Some(0));
        assert_eq!(retreat(0, 3, Repeat::Off), None);
        assert_eq!(retreat(0, 3, Repeat::All), Some(2));
        assert_eq!(playlist_osd(0, 3), "1/3");
        assert_eq!(playlist_osd(2, 3), "3/3");
    }

    /// One dealt cycle holds every item of the list but the one on screen, in
    /// some order; the same seed always deals the same order and another seed
    /// deals another, while a list of one has nothing to deal at all.
    #[test]
    fn a_cycle_is_dealt_over_the_whole_list_once() {
        let mut seed = 0x243f_6a88_85a3_08d3;
        let cycle = deal_cycle(5, 2, &mut seed);
        let mut sorted = cycle.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 3, 4]);
        let mut same = 0x243f_6a88_85a3_08d3;
        assert_eq!(deal_cycle(5, 2, &mut same), cycle);
        let mut other = 0x8def_8291_d712_b3bf;
        assert_ne!(deal_cycle(5, 2, &mut other), cycle);
        let mut empty = 7;
        assert!(deal_cycle(1, 0, &mut empty).is_empty());
        assert!(deal_cycle(0, 0, &mut empty).is_empty());
    }

    /// With Random on the next key walks the drawn cycle: every other item of
    /// the list plays exactly once, and a list that does not repeat then says
    /// it is at the end. The previous key returns along the walked steps, and
    /// the item stepped past stands open for the draw again. A repeating list
    /// deals a fresh cycle where the old one ran out, and turning Random off
    /// leaves the plain order to resume from the item on screen.
    #[test]
    fn the_shuffled_list_walks_its_cycle_and_returns_its_steps() {
        let directory = scratch("fvid-player-shuffle", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m", "four.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three", "four"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!(player.name, "one.y4m");
        player.apply(Control::Shuffle);
        assert_eq!(player.osd.as_ref().unwrap().0, "Shuffle on");
        let mut drawn = Vec::new();
        for _ in 0..3 {
            player.apply(Control::Next);
            drawn.push(player.name.clone());
        }
        let mut seen = drawn.clone();
        seen.push("one.y4m".to_owned());
        seen.sort();
        assert_eq!(
            seen,
            ["four.y4m", "one.y4m", "three.y4m", "two.y4m"],
            "{drawn:?}"
        );
        player.apply(Control::Next);
        assert_eq!(player.osd.as_ref().unwrap().0, "End of playlist");
        player.apply(Control::Previous);
        assert_eq!(player.name, drawn[1]);
        player.apply(Control::Next);
        assert_eq!(player.name, drawn[2]);
        player.repeat = Repeat::All;
        player.apply(Control::Next);
        assert_ne!(player.name, drawn[2]);
        player.apply(Control::Shuffle);
        assert_eq!(player.osd.as_ref().unwrap().0, "Shuffle off");
        let before = player.index;
        player.apply(Control::Next);
        assert_eq!(player.index, (before + 1) % 4);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A shuffled list continues by itself along the cycle: an item running
    /// out starts the next draw, not the next number, and the list ends when
    /// the cycle has played everything once.
    #[test]
    fn a_shuffled_item_running_out_walks_the_cycle() {
        let directory = scratch("fvid-player-shuffle-auto", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        player.play_index(0);
        player.shuffle = true;
        player.upcoming = deal_cycle(3, 0, &mut 99);
        player.walked = vec![0];
        player.continue_queue();
        let first = player.index;
        assert_ne!(first, 0);
        player.continue_queue();
        let second = player.index;
        assert_ne!(second, first);
        let mut seen = [0, first, second];
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2]);
        player.continue_queue();
        assert!(player.ended);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A session started out with `--random` holds the cycle dealt to the
    /// queue the command line named: the walk begins at the item on screen and
    /// every other item stands open for a draw before the first step is taken.
    #[test]
    fn a_random_start_holds_the_cycle_the_queue_was_dealt() {
        let directory = scratch("fvid-player-random-start", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            shuffle: true,
            ..Default::default()
        };
        player.start_shuffle();
        assert_eq!(player.walked, [0]);
        assert_eq!(player.upcoming.len(), 2);
        player.play_index(0);
        player.apply(Control::Next);
        assert_ne!(player.index, 0);
        let mut seen = vec![0, player.index];
        player.apply(Control::Next);
        seen.push(player.index);
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2]);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn repeat_cycles_in_vlc_order() {
        let mut repeat = Repeat::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            repeat = cycle_repeat(repeat);
            seen.push(repeat);
        }
        assert_eq!(seen, [Repeat::All, Repeat::One, Repeat::Off, Repeat::All]);
        assert_eq!(repeat_osd(Repeat::One), "Repeat one");
    }

    /// A list is only as good as the files it points at: comments, notes,
    /// network entries and missing files are passed over, and everything else
    /// is looked for beside the list rather than beside the player.
    #[test]
    fn a_list_names_the_files_that_are_actually_there() {
        let directory = scratch("fvid-player-playlist", &["a.mp4", "b.webm"]);
        let list = directory.join("queue.m3u");
        std::fs::write(
            &list,
            "#EXTM3U\n#EXTINF:10,a\na.mp4\n#EXTINF:20,b\nmissing.mp4\nhttps://example.test/b.webm\n",
        )
        .unwrap();
        let names: Vec<String> = expand_inputs(&[list])
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.mp4"]);

        // A PLS keeps its paths behind numbered keys, next to titles that are
        // not paths at all.
        let pls = directory.join("queue.pls");
        std::fs::write(
            &pls,
            "[playlist]\nFile1=b.webm\nTitle1=B\nNumberOfEntries=1\n",
        )
        .unwrap();
        let entries = expand_inputs(&[pls]);
        assert_eq!(entries, [directory.join("b.webm")]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unusable_list_is_opened_as_itself_so_the_error_says_why() {
        let directory = scratch("fvid-player-empty-playlist", &[]);
        let list = directory.join("empty.m3u");
        std::fs::write(&list, "#EXTM3U\n").unwrap();
        let inputs = vec![list];
        assert_eq!(expand_inputs(&inputs), inputs);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Stepping onto a file that will not open leaves the picture where it was,
    /// rather than a blank window with a stale index.
    #[test]
    fn a_step_onto_a_broken_item_keeps_the_picture() {
        let directory = scratch("fvid-player-broken-playlist", &["a.mp4"]);
        let mut player = Player {
            queue: vec![directory.join("a.mp4"), directory.join("gone.mp4")],
            ..Default::default()
        };
        player.play_index(1);
        assert_eq!(player.index, 0);
        assert!(player.error.is_some());

        player.step_queue(false);
        assert_eq!(player.index, 0);

        player.step_queue(true);
        assert_eq!(player.osd.as_ref().unwrap().0, "First item");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn folders_expand_in_name_order_and_ignore_unrelated_files() {
        let directory = scratch("fvid-folder-queue", &["b.mp4", "a.Y4M", "notes.txt"]);
        std::fs::create_dir(directory.join("subfolder.mp4")).unwrap();
        assert_eq!(expand_inputs(&[directory.clone()]), [directory.join("a.Y4M"), directory.join("b.mp4")]);
        let list = directory.join("queue.m3u8");
        std::fs::write(&list, "\u{feff}#EXTM3U\nb.mp4\na.Y4M\n").unwrap();
        assert_eq!(expand_inputs(&[list]), [directory.join("b.mp4"), directory.join("a.Y4M")]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn next_file_is_cached_during_current_playback_and_reused_at_eof() {
        let directory = scratch("fvid-prefetch-next", &[]);
        let first = directory.join("a.y4m");
        let second = directory.join("b.y4m");
        std::fs::write(&first, y4m(100)).unwrap();
        std::fs::write(&second, y4m(2)).unwrap();
        let mut player = Player { no_audio: true, ..Default::default() };
        player.open_queue(vec![first.clone(), second.clone()]);
        player.prefetch_next();
        assert_eq!(player.opened.as_ref(), Some(&first));
        assert!(player.playback.is_some());
        let next = player.next_file.take().unwrap();
        let mut cached = next.source.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        // The cache may use RAM or a spool. Require the complete original bytes
        // and reuse after unlinking the source, independently of storage policy.
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut cached.0, &mut bytes).unwrap();
        assert_eq!(bytes, y4m(2));
        std::io::Seek::seek(&mut cached.0, std::io::SeekFrom::Start(0)).unwrap();
        let (send, source) = std::sync::mpsc::channel();
        send.send(Ok(cached)).unwrap();
        player.next_file = Some(super::NextFile { path: second.clone(), source, cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)) });
        std::fs::remove_file(&second).unwrap();
        player.continue_queue();
        assert_eq!(player.index, 1);
        assert_eq!(player.opened.as_ref(), Some(&second));
        assert!(player.error.is_none(), "{:?}", player.error);
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn prefetch_tracks_shuffle_and_queue_replacement() {
        let directory = scratch("fvid-prefetch-shuffle", &[]);
        let paths: Vec<_> = ["a.y4m", "b.y4m", "c.y4m"].iter().map(|name| directory.join(name)).collect();
        for path in &paths { std::fs::write(path, y4m(2)).unwrap(); }
        let mut player = Player { no_audio: true, ..Default::default() };
        player.open_queue(paths.clone());
        player.shuffle = true;
        player.upcoming = vec![1, 2];
        player.prefetch_next();
        assert_eq!(player.next_file.as_ref().unwrap().path, paths[2]);
        player.upcoming.clear();
        player.repeat = super::Repeat::All;
        player.prefetch_next();
        let predicted = player.next_file.as_ref().unwrap().path.clone();
        let target = player.shuffle_forward().unwrap();
        assert_eq!(predicted, paths[target]);
        player.repeat = super::Repeat::Off;
        let cancelled = player.next_file.as_ref().unwrap().cancelled.clone();
        player.open_queue(vec![paths[1].clone()]);
        assert!(cancelled.load(std::sync::atomic::Ordering::Acquire));
        player.prefetch_next();
        assert!(player.next_file.is_none());
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Two frames of 2×2 4:2:0: the smallest thing the reader accepts.
    fn y4m(frames: usize) -> Vec<u8> {
        let mut bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n".to_vec();
        for _ in 0..frames {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[0u8; 6]);
        }
        bytes
    }

    #[test]
    fn a_late_window_catches_up_and_preserves_the_final_picture() {
        let mut reader = NativeReader::without_memory_limit(std::io::Cursor::new(y4m(4))).unwrap();
        assert!(reader.read_frame().unwrap());
        let playback = Playback::start(reader, None);
        let until = Instant::now() + Duration::from_secs(5);
        while playback.filled() < 4 {
            assert!(Instant::now() < until, "decoder did not fill the queue");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut player = Player {
            playback: Some(playback),
            buffering: false,
            deadline: Instant::now() - Duration::from_secs(1),
            period: Duration::from_millis(40),
            ..Default::default()
        };
        let ctx = eframe::egui::Context::default();
        player.present(&ctx);
        let final_serial = player
            .presented
            .as_ref()
            .expect("final picture was shown")
            .serial;
        assert!(final_serial >= 3, "late pictures were not discarded");
        // EOF may arrive after the pictures; it must still complete the queue.
        while !player.ended {
            assert!(Instant::now() < until, "EOF was lost during catch-up");
            player.present(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(player.presented.as_ref().unwrap().serial, final_serial);
    }

    /// Reaching the end of an item is what moves the list on; the last item of
    /// a list that does not repeat stops on its own picture.
    #[test]
    fn the_end_of_an_item_starts_the_next_one() {
        let directory = scratch("fvid-player-queue", &[]);
        for name in ["one.y4m", "two.y4m"] {
            std::fs::write(directory.join(name), y4m(2)).unwrap();
        }
        let mut player = Player {
            queue: vec![directory.join("one.y4m"), directory.join("two.y4m")],
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!(player.name, "one.y4m");

        player.continue_queue();
        assert_eq!((player.name.as_str(), player.index), ("two.y4m", 1));
        assert!(!player.ended);

        player.continue_queue();
        assert_eq!(player.name, "two.y4m");
        assert!(player.ended);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn repeating_one_item_reopens_it() {
        let directory = scratch("fvid-player-repeat-one", &[]);
        std::fs::write(directory.join("only.y4m"), y4m(1)).unwrap();
        let mut player = Player {
            queue: vec![directory.join("only.y4m")],
            repeat: Repeat::One,
            ..Default::default()
        };
        player.play_index(0);
        player.continue_queue();
        assert_eq!(
            (player.name.as_str(), player.index, player.ended),
            ("only.y4m", 0, false)
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file with two AAC tracks behind one video: the list comes from the
    /// container, the key moves the sound onto the other track, and the picture
    /// is left alone. Reaching the end of the list wraps back to the first track.
    #[test]
    fn a_file_with_two_audio_tracks_walks_between_them() {
        let directory = scratch("fvid-player-audio-tracks", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!((player.audio_tracks.len(), player.audio_track), (2, 0));

        player.apply(Control::AudioTrack(true));
        assert_eq!(player.audio_track, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 2/2 1 ch 32000 Hz");
        assert_eq!(player.name, "two-audio.mp4");

        player.apply(Control::AudioTrack(true));
        assert_eq!(player.audio_track, 0);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 1/2 1 ch 48000 Hz");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The named MP4, whose three audio tracks say different amounts about
    /// themselves: the key quotes the title for the first, the language for the
    /// second and nothing but the layout for the third, and the subtitle list
    /// does the same for the one track the muxer titled.
    /// `tests/fixtures/tracks/named.mp4`, whose command `tests/mp4.rs` records.
    #[test]
    fn the_audio_list_says_what_the_file_says_about_each_track() {
        let directory = scratch("fvid-player-named-tracks", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.audio_tracks.len(), 3);

        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 2/3 fre · 1 ch 32000 Hz"
        );
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 3/3 1 ch 44100 Hz");
        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 1/3 Первая · 1 ch 48000 Hz"
        );

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 mov_text");
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 Титры");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The same two lists out of a Matroska file, where the title and the
    /// language are elements of their own and the picture track, which states
    /// only that nothing was said, is left out of both.
    /// `tests/fixtures/tracks/named.mkv`, whose command the container's own test
    /// records.
    #[test]
    fn the_matroska_lists_say_what_the_file_says_about_each_track() {
        let directory = scratch("fvid-player-named-matroska-tracks", &[]);
        let path = directory.join("named.mkv");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/tracks/named.mkv")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            (player.audio_tracks.len(), player.subtitle_sources.len()),
            (2, 2)
        );

        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 2/2 fre · 1 ch 32000 Hz"
        );
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 Титры");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Two text tracks behind one picture: opening the file finds both, the key
    /// moves the text onto the other one, and the list wraps at either end.
    #[test]
    fn a_file_with_two_subtitle_tracks_walks_between_them() {
        let directory = scratch("fvid-player-subtitle-tracks", &[]);
        let path = directory.join("text-tracks.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        assert_eq!(player.subtitle_line(seconds(700)), Some("plain first"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        assert_eq!(player.subtitle_line(seconds(700)), Some("other track"));
        assert_eq!(player.subtitle_line(seconds(1_400)), Some("tagged words"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        player.apply(Control::SubtitleTrack(false));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.name, "text-tracks.mkv");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An ASS track of the file itself, in the window: the list shows it under
    /// the title the file gives it, the line painted is the text of its blocks
    /// with the tags left out, and the key walks it beside the SubRip track of
    /// the same file. `tests/fixtures/subtitles/ass-track.mkv`, whose command
    /// the reader's own test records.
    #[test]
    fn an_ass_track_of_the_file_is_listed_painted_and_walked() {
        let directory = scratch("fvid-player-ass-track", &[]);
        let path = directory.join("ass-track.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/subtitles/ass-track.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        // The ASS track is first in container order, so it is also the default.
        assert_eq!(
            player.subtitle_line(seconds(700)),
            Some("First\nsecond line")
        );
        assert_eq!(
            player.subtitle_line(seconds(2_500)),
            Some("Hello, world with commas")
        );

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        assert_eq!(player.subtitle_line(seconds(1_500)), Some("One, two"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 События");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The same list built out of an MP4, which the Matroska reader cannot open:
    /// the file's first bytes pick the other reader, and from there the tracks,
    /// the labels and the key behave as they do for a Matroska file.
    #[test]
    fn a_file_with_two_mov_text_tracks_walks_between_them() {
        let directory = scratch("fvid-player-mov-text-tracks", &[]);
        let path = directory.join("mov-text-tracks.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/subtitles/mov-text-tracks.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        assert_eq!(player.subtitle_line(seconds(150)), Some("first line"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 mov_text");
        // The other track holds nothing at 150 ms and its own two lines later,
        // the last of them stripped of the markup the encoder moved into a box.
        assert_eq!(player.subtitle_line(seconds(150)), None);
        assert_eq!(player.subtitle_line(seconds(250)), Some("other track"));
        assert_eq!(player.subtitle_line(seconds(450)), Some("tagged words"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.name, "mov-text-tracks.mp4");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A sidecar the user placed is the first choice; the file's own tracks stay
    /// behind it in the list rather than replacing it.
    #[test]
    fn a_sidecar_sits_in_front_of_the_tracks_in_the_file() {
        let directory = scratch("fvid-player-subtitle-sidecar-and-tracks", &["clip.srt"]);
        std::fs::write(
            directory.join("clip.mkv"),
            include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        std::fs::write(
            directory.join("clip.srt"),
            "1\n00:00:00,600 --> 00:00:00,800\nfrom the sidecar\n",
        )
        .unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.mkv")],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 3);
        assert_eq!(player.subtitle_name, "clip.srt");
        assert_eq!(player.subtitle_line(seconds(700)), Some("from the sidecar"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/3 UTF-8");
        assert_eq!(player.subtitle_line(seconds(700)), Some("plain first"));
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A SAMI file — the Windows-era caption, where every line carries its own
    /// time in the tag that opens it — is a sidecar like the others, so the line
    /// reaches the screen at the milliseconds its `Begin` and `End` name.
    #[test]
    fn a_sami_sidecar_captions_the_item_beside_which_it_lies() {
        let directory = scratch("fvid-player-sami-sidecar", &[]);
        std::fs::write(
            directory.join("clip.mkv"),
            include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        std::fs::write(
            directory.join("clip.smi"),
            "<SAMI>\n<HEAD><TITLE>Clip</TITLE></HEAD>\n<BODY>\n\
             <p class=eng Begin=500 End=1500>from the sami file\n",
        )
        .unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.mkv")],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_name, "clip.smi");
        assert_eq!(
            player.subtitle_line(seconds(1_000)),
            Some("from the sami file")
        );
        assert_eq!(
            player.subtitle_line(seconds(2_000)),
            None,
            "the caption's own end is what takes it away"
        );
        // The file's two text tracks are still there, behind it.
        assert_eq!(player.subtitle_sources.len(), 3);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_subtitle_key_needs_a_choice_to_do_anything() {
        let mut player = Player {
            cues: subtitles::parse("1\n00:00:00,500 --> 00:00:01,000\none track\n"),
            subtitle_name: "clip.srt".to_owned(),
            subtitle_sources: vec![SubtitleSource {
                label: "clip.srt".to_owned(),
                cues: subtitles::parse("1\n00:00:00,500 --> 00:00:01,000\none track\n"),
            }],
            ..Default::default()
        };
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/1");
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.subtitle_line(seconds(700)), Some("one track"));

        let mut player = Player::default();
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No subtitles");
    }

    #[test]
    fn a_snapshot_is_named_by_the_time_it_was_taken() {
        let video = PathBuf::from("/tmp/clip.mp4");
        assert_eq!(
            snapshot_name(&video, seconds(3_723_456)),
            PathBuf::from("/tmp/clip-01h02m03s456.png")
        );
        assert_eq!(
            snapshot_name(&video, Duration::ZERO),
            PathBuf::from("/tmp/clip-00h00m00s000.png")
        );
    }

    #[test]
    fn a_snapshot_writes_the_picture_on_screen() {
        let directory = scratch("fvid-player-snapshot", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let mut player = Player {
            opened: Some(video),
            presented: Some(Frame {
                pixels: Pixels::Rgb((1..=12).collect()),
                dimensions: [2, 2],
                period: Duration::from_millis(40),
                interval: None,
                pts: None,
                generation: 0,
                serial: 1,
                rotation: 0,
            }),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Saved clip-00h00m00s000.png"
        );
        let file = std::fs::read(directory.join("clip-00h00m00s000.png")).expect("written");
        assert_eq!(
            &file[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        // IHDR starts at the tenth byte with the two 32-bit dimensions.
        assert_eq!(&file[16..24], &[0, 0, 0, 2, 0, 0, 0, 2]);
        let alternate = directory.join("shots/nested");
        player.snapshot_dir = Some(alternate.clone());
        player.take_snapshot();
        assert_eq!(std::fs::read(alternate.join("clip-00h00m00s000.png")).unwrap(), file);

        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A two-entry cube that turns green end for end and leaves the other two
    /// channels alone — a grade whose every step is a per-channel table.
    const INVERT_GREEN: &str = "LUT_1D_SIZE 2
0.0 1.0 0.0
1.0 0.0 1.0
";

    /// The scanlines a stored-block PNG carries, filter bytes removed. Every
    /// snapshot in these tests is written stored and unfiltered, so the picture
    /// can be read back out of the file it was saved as.
    fn scanlines(file: &[u8]) -> Vec<u8> {
        let (width, height) = (
            usize::try_from(u32::from_be_bytes(file[16..20].try_into().unwrap())).unwrap(),
            usize::try_from(u32::from_be_bytes(file[20..24].try_into().unwrap())).unwrap(),
        );
        let row = width * 3 + 1;
        let mut at = 8;
        let idat = loop {
            let len = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
            if &file[at + 4..at + 8] == b"IDAT" {
                break &file[at + 8..at + 8 + len];
            }
            at += 12 + len;
        };
        assert_eq!(&idat[..2], &[0x78, 0x01], "stored deflate, no dictionary");
        // The stream ends with its four-byte Adler checksum.
        let rows = &idat[7..idat.len() - 4];
        assert_eq!(rows.len(), row * height, "more than one stored block");
        let mut rgb = Vec::with_capacity(width * height * 3);
        for (line, number) in rows.chunks_exact(row).zip(0..height) {
            assert_eq!(line[0], 0, "row {number} was filtered");
            rgb.extend_from_slice(&line[1..]);
        }
        rgb
    }

    /// Planes are the one form the window shows without touching its pixels, so
    /// they are also the form a grade can ride to the shader with. The snapshot
    /// has to finish that grade itself: the same planes saved with nothing owed
    /// and with a green turned end for end come out as one picture's scanlines
    /// with their green inverted and no other channel moved.
    #[test]
    fn a_snapshot_converts_planes_before_writing_them() {
        let directory = scratch("fvid-player-snapshot-planes", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let shown = |grade: Option<crate::color::Grade>| {
            let planes = Planar8 {
                width: 2,
                height: 2,
                chroma_width: 1,
                chroma_height: 1,
                y: vec![16, 128, 200, 235],
                cb: vec![128],
                cr: vec![128],
                colour: Default::default(),
            };
            let player = Player {
                opened: Some(video.clone()),
                presented: Some(Frame {
                    pixels: Pixels::Planar(Arc::new(planes), grade.map(Arc::new)),
                    dimensions: [2, 2],
                    period: Duration::from_millis(40),
                    interval: None,
                    pts: None,
                    generation: 0,
                    serial: 1,
                    rotation: 0,
                }),
                ..Default::default()
            };
            player.picture().expect("planes become a picture")
        };
        let plain = shown(None);
        assert_eq!(&plain[16..24], &[0, 0, 0, 2, 0, 0, 0, 2]);
        let signal = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let grade = crate::color::Grade::new(
            signal,
            &HdrMetadata::default(),
            crate::color::Settings::video(DisplayTarget::sdr(240.0)),
            Some(Lut::from_cube(INVERT_GREEN).expect("a written cube is a cube")),
        );
        // The route this is about: a grade the shader carries, which is why a
        // plane picture would reach the window still owing it.
        assert!(
            grade.is_shader_look(),
            "a plane picture that never keeps a grade tests nothing here"
        );
        let graded = shown(Some(grade));
        let (plain, graded) = (scanlines(&plain), scanlines(&graded));
        assert_eq!(plain.len(), graded.len());
        for (before, after) in plain
            .as_chunks::<3>()
            .0
            .iter()
            .zip(graded.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let want = if channel == 1 {
                    255 - u16::from(before[1])
                } else {
                    u16::from(before[channel])
                };
                assert_eq!(
                    u16::from(after[channel]),
                    want,
                    "channel {channel} of a graded plane snapshot"
                );
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The snapshot key is the one place this tool writes colour out as a file,
    /// so the picture it saves has to be the graded one. A two-node cube that
    /// turns green end for end is read off text, handed to the thread that shows
    /// a synthetic stream, and the frame that thread shows is saved: the saved
    /// file's own scanlines come out byte for byte as that frame, and that
    /// frame's green is what the cube's text says of the ungraded picture. The
    /// file's own header is asked as well, because a stored, unfiltered PNG
    /// carries one flat run of bytes and a picture encoded at the wrong size
    /// still reads back as the same pixels. Three mutations die here: rows
    /// labelled with a filter that was never applied, byte tables built without
    /// the cube, and dimensions swapped on the way into the encoder.
    #[test]
    fn a_saved_picture_is_the_one_the_grade_made() {
        fn stream() -> Vec<u8> {
            let mut bytes = b"YUV4MPEG2 W4 H2 F60:1 Ip C420jpeg\n".to_vec();
            for frame in 0..2u8 {
                bytes.extend_from_slice(b"FRAME\n");
                for y in 0..2u8 {
                    for x in 0..4u8 {
                        bytes.push(16 + x * 40 + y * 7 + frame * 3);
                    }
                }
                bytes.push(64 + frame);
                bytes.push(190 - frame);
            }
            bytes
        }
        fn reader() -> NativeReader<std::io::Cursor<Vec<u8>>> {
            let mut reader = NativeReader::without_memory_limit(std::io::Cursor::new(stream()))
                .expect("a written stream reads");
            assert!(reader.read_frame().expect("a first frame"));
            reader
        }
        fn shown(player: &mut Playback) -> Frame {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "playback stalled");
                match player.poll() {
                    Some(Event::Frame(frame)) => return frame,
                    Some(Event::Error(error)) => panic!("{error}"),
                    Some(Event::Ended(_)) => panic!("the stream ended before its first frame"),
                    None => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }
        fn rgb(pixels: &Pixels, budget: usize) -> Vec<u8> {
            match pixels {
                #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                Pixels::Surface(surface) => {
                    let p = crate::playback_native::surface_to_packed(&surface.surface, surface.colour).unwrap().rotated(surface.rotation).unwrap();
                    let mut rgb = Vec::new(); p.to_rgb(&mut rgb, budget).unwrap();
                    if let Some(grade) = &surface.grade { grade.apply(&mut rgb); } rgb
                },
                Pixels::Packed(planes, grade) => {
                    let mut rgb = Vec::new();
                    planes.to_rgb(&mut rgb, budget).unwrap();
                    if let Some(grade) = grade { grade.apply(&mut rgb); }
                    rgb
                }
                Pixels::Rgb(rgb) => rgb.clone(),
                Pixels::Rgba(rgba) => crate::playback_native::rgba_to_rgb_on_black(rgba, budget).unwrap(),
                Pixels::Planar(planes, grade) => {
                    let mut rgb = Vec::new();
                    crate::playback_native::planar8_to_rgb(planes, &mut rgb, budget)
                        .expect("converted");
                    // What the window shows is the planes and whatever grade they
                    // kept for the shader.
                    if let Some(grade) = grade {
                        grade.apply(&mut rgb);
                    }
                    rgb
                }
            }
        }

        let signal = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let settings = crate::color::Settings::video(crate::color::DisplayTarget::sdr(240.0));
        assert!(
            crate::color::Grade::new(signal, &HdrMetadata::default(), settings, None).is_identity(),
            "these settings already move this stream on their own, so a cube would not be the only change"
        );
        let grade = crate::color::Grade::new(
            signal,
            &HdrMetadata::default(),
            settings,
            Some(Lut::from_cube(INVERT_GREEN).expect("a written cube is a cube")),
        );
        let budget = reader().rgb_budget();
        let mut plain = Playback::start(reader(), None);
        let mut graded = Playback::start(reader(), Some(grade));
        let untouched = rgb(&shown(&mut plain).pixels, budget);
        let frame = shown(&mut graded);
        let dimensions = frame.dimensions;
        let painted = rgb(&frame.pixels, budget);
        assert_ne!(untouched, painted, "the cube reached no pixels");
        assert_eq!(dimensions, [4, 2]);
        for (pixel, out) in untouched
            .as_chunks::<3>()
            .0
            .iter()
            .zip(painted.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let want = if channel == 1 {
                    255 - i16::from(pixel[1])
                } else {
                    i16::from(pixel[channel])
                };
                let step = (i16::from(out[channel]) - want).abs();
                assert!(
                    step <= 1,
                    "channel {channel}: {} shown as {}, the cube's own text says {want}",
                    pixel[channel],
                    out[channel]
                );
            }
        }

        let directory = scratch("fvid-player-snapshot-graded", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let mut player = Player {
            opened: Some(video),
            presented: Some(frame),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        let saved = player.osd.as_ref().unwrap().0.clone();
        let name = saved
            .strip_prefix("Saved ")
            .unwrap_or_else(|| panic!("the snapshot key said {saved}"));
        let file = std::fs::read(directory.join(name)).expect("written");
        // A stored, unfiltered PNG carries its pixels as one flat run, so a file
        // written for the wrong size still hands back the same bytes: the header
        // has to be asked what picture it says, not only the scanlines.
        let saved = [
            usize::try_from(u32::from_be_bytes(file[16..20].try_into().unwrap())).unwrap(),
            usize::try_from(u32::from_be_bytes(file[20..24].try_into().unwrap())).unwrap(),
        ];
        assert_eq!(
            saved, dimensions,
            "the file says a different size from the picture shown"
        );
        assert_eq!(
            scanlines(&file),
            painted,
            "the file holds a different picture from the one shown"
        );
        assert_ne!(
            scanlines(&file),
            untouched,
            "the file holds the ungraded picture"
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_snapshot_without_a_picture_says_so() {
        let mut player = Player::default();
        player.apply(Control::Snapshot);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to save");

        let mut player = Player {
            opened: Some(PathBuf::from("/tmp/clip.mp4")),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to save");
    }

    /// Arguments are strings, so a queue of two files also has to survive the
    /// options written between them.
    #[test]
    fn play_options_read_the_queue_and_their_own_values() {
        let parsed = play_args(&[
            "a.mp4",
            "--start-time",
            "1:30",
            "--stop-time=95.5",
            "--rate",
            "2.5",
            "b.webm",
        ])
        .expect("options and inputs mix");
        assert_eq!(
            parsed.paths,
            [PathBuf::from("a.mp4"), PathBuf::from("b.webm")]
        );
        assert_eq!(parsed.start, Some(seconds(90_000)));
        assert_eq!(parsed.stop, Some(seconds(95_500)));
        assert_eq!(parsed.rate_milli, 2_500);

        let parsed = play_args(&["clip.mp4"]).expect("one input, no options");
        assert_eq!(parsed.paths, [PathBuf::from("clip.mp4")]);
        assert_eq!(
            (parsed.start, parsed.stop, parsed.rate_milli),
            (None, None, 1_000)
        );
    }

    #[test]
    fn play_options_refuse_what_they_cannot_honour() {
        // A flag without anything after it, a rate outside VLC's ladder, a clock
        // with too many parts, and bounds that cross over.
        assert!(play_args(&["--start-time"]).is_err());
        assert!(play_args(&["a.mp4", "--rate"]).is_err());
        assert!(play_args(&["--rate", "9"]).is_err());
        assert!(play_args(&["--start-time", "1:2:3:4"]).is_err());
        assert!(play_args(&["--start-time", "70", "--stop-time", "60"]).is_err());
        assert!(play_args(&["--toggle", "a.mp4"]).is_err());
        assert_eq!(
            play_args(&["a=b.mp4"])
                .expect("a name with a sign")
                .paths
                .len(),
            1
        );
    }

    #[test]
    fn the_end_clock_flips_between_length_and_what_is_left() {
        let (at, total) = (Duration::from_secs(2_537), Duration::from_secs(7_083));
        assert_eq!(end_clock(at, total, false), "1:58:03");
        assert_eq!(end_clock(at, total, true), "\u{2212}1:15:46");
        assert_eq!(end_clock(total * 2, total, true), "\u{2212}0:00");
    }

    #[test]
    fn a_moment_falls_in_the_last_chapter_started() {
        let chapters = [
            ChapterMark {
                start: Duration::from_secs(10),
                title: "Opening".to_owned(),
            },
            ChapterMark {
                start: Duration::from_secs(60),
                title: String::new(),
            },
        ];
        assert_eq!(chapter_at(&chapters, Duration::from_secs(5)), None);
        assert_eq!(
            chapter_at(&chapters, Duration::from_secs(10)),
            Some((1, "Opening"))
        );
        assert_eq!(
            chapter_at(&chapters, Duration::from_secs(90)),
            Some((2, ""))
        );
    }

    #[test]
    fn clocks_are_seconds_or_a_ladder_of_them() {
        assert_eq!(parse_clock("90").expect("plain seconds"), seconds(90_000));
        assert_eq!(parse_clock("1:30").expect("minutes"), seconds(90_000));
        assert_eq!(
            parse_clock("01:02:03.5").expect("hours"),
            seconds(3_723_500)
        );
        assert!(parse_clock("-5").is_err());
        assert!(parse_clock("").is_err());
        assert!(parse_clock("abc").is_err());
    }

    #[test]
    fn swap_uses_packet_timestamps_and_does_not_colour_container_headers() {
        let packets = vec![
            (900, 1000, Duration::ZERO, Duration::from_secs(1)),
            (100, 200, Duration::from_secs(1), Duration::from_secs(2)),
            (300, 400, Duration::from_secs(4), Duration::from_secs(5)),
        ];
        let ranges = super::cached_time_ranges(
            &packets,
            &[(900, 1100), (300, 400)],
            Duration::from_secs(10),
        );
        assert_eq!(ranges, vec![[0.0, 0.1], [0.4, 0.5]]);
        assert!(
            super::cached_time_ranges(&packets, &[(1000, 1100)], Duration::from_secs(10))
                .is_empty()
        );
    }

    #[test]
    fn cache_lines_separate_ram_and_disk_and_retain_ranges_after_seeks() {
        use std::io::{Read, Seek, SeekFrom};
        let directory = scratch("fvid-cache-line-tiers", &[]);
        let path = directory.join("synthetic.bin");
        std::fs::write(&path, vec![0x5a; 4096]).unwrap();
        for memory in [true, false] {
            let mut spool = if memory {
                crate::playback_spool::Spool::in_memory(&path, 4096)
            } else { crate::playback_spool::Spool::open(&path, 4096) }.unwrap();
            let handle = spool.handle();
            assert_eq!(handle.is_memory(), memory);
            let mut player = Player { duration: Some(Duration::from_secs(4)),
                spool: Some(handle), cache_packets: vec![
                    (0, 1024, Duration::ZERO, Duration::from_secs(1)),
                    (2048, 3072, Duration::from_secs(2), Duration::from_secs(3)),
                ], ..Default::default() };
            let mut bytes = Vec::new(); spool.read_to_end(&mut bytes).unwrap();
            for at in [2048, 0, 3072, 0] {
                spool.seek(SeekFrom::Start(at)).unwrap();
                let mut byte = [0]; spool.read_exact(&mut byte).unwrap();
                assert_eq!(byte, [0x5a]);
                assert_eq!(player.storage_ranges(memory), vec![[0.0, 0.25], [0.5, 0.75]]);
                assert!(player.storage_ranges(!memory).is_empty());
            }
            player.spool = None;
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn neighbouring_cached_frames_form_one_solid_strip() {
        let packets = vec![(0, 10, Duration::ZERO, Duration::from_nanos(33_333_333)),
            (10, 20, Duration::from_nanos(33_333_334), Duration::from_nanos(66_666_667))];
        let ranges = super::cached_time_ranges(&packets, &[(0, 20)], Duration::from_secs(1));
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0][0], 0.0);
        assert!(ranges[0][1] > 0.066);
    }

    #[test]
    fn slow_source_shows_buffering_and_resumes_when_frames_arrive() {
        let directory = scratch("fvid-stall-buffer", &[]);
        let path = directory.join("cloud-startup.y4m");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/playback-errors/cloud-startup.y4m")).unwrap();
        let mut player = Player { no_audio: true, ..Default::default() };
        player.open(path).unwrap();
        player.buffering = false;
        let now = Instant::now();
        player.deadline = now;
        player.buffer_on_stall(now);
        assert!(!player.buffering, "normal frame interval is not a stall");
        player.buffer_on_stall(now + Duration::from_secs(1));
        assert!(player.buffering);
        assert!(player.loading_since.is_some());
        assert_eq!(player.opening_stage, "Ожидание данных файла");
        let ctx = eframe::egui::Context::default();
        let until = Instant::now() + Duration::from_secs(5);
        while player.buffering {
            assert!(Instant::now() < until, "buffering never resumed");
            player.present(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.loading_since.is_none());
        assert!(player.error.is_none());
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cloud_startup_stays_loading_until_delayed_source_arrives() {
        let directory = scratch("fvid-cloud-startup", &[]);
        let path = directory.join("cloud-startup.y4m");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/playback-errors/cloud-startup.y4m")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let mut player = Player { asynchronous_open: true, no_audio: true,
            prepared_source: Some(receive), ..Default::default() };
        player.begin_open(path.clone());
        let ctx = eframe::egui::Context::default();
        // The source is deliberately unavailable; polling must still return.
        for _ in 0..3 { player.poll_open(&ctx); }
        assert!(player.opening.is_some());
        assert!(player.loading_since.is_some());
        assert!(player.buffering);
        assert!(player.error.is_none());
        assert!(player.playback.is_none());
        let prepared = super::prepare_source(&path, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert!(matches!(&prepared.0, crate::playback_spool::Source::Spooled(_)));
        send.send(Ok(prepared)).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while player.opening.is_some() {
            assert!(Instant::now() < until);
            player.poll_open(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.playback.is_some());
        assert!(player.loading_since.is_none());
        assert_eq!(player.dimensions, [2, 2]);
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn background_open_returns_immediately_and_can_be_superseded() {
        let directory = scratch("fvid-background-open", &[]);
        let path = directory.join("clip.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            asynchronous_open: true,
            no_audio: true,
            ..Default::default()
        };
        player.try_open(directory.join("missing.mp4"));
        player.try_open(path.clone());
        assert!(player.opening.is_some());
        assert!(player.playback.is_none());
        let ctx = eframe::egui::Context::default();
        let until = Instant::now() + Duration::from_secs(5);
        while player.opening.is_some() {
            assert!(Instant::now() < until);
            player.poll_open(&ctx);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.opened, Some(path));
        assert!(player.playback.is_some());
        assert!(!player.cache_packets.is_empty());
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn background_reopen_replaces_a_full_decoder_queue() {
        let directory = scratch("fvid-background-reopen", &[]);
        let path = directory.join("clip.y4m");
        std::fs::write(&path, y4m(120)).unwrap();
        let mut player = Player {
            asynchronous_open: true,
            no_audio: true,
            ..Default::default()
        };
        let ctx = eframe::egui::Context::default();
        for _ in 0..3 {
            player.open_queue(vec![path.clone()]);
            let until = Instant::now() + Duration::from_secs(5);
            while player.opening.is_some() {
                assert!(Instant::now() < until, "reopen did not complete");
                player.poll_open(&ctx);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(player.error.is_none(), "{:?}", player.error);
            let playback = player.playback.as_ref().unwrap();
            while playback.filled() < playback.depth() {
                assert!(Instant::now() < until, "decoder queue did not fill");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(player.opened.as_ref(), Some(&path));
        }
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_live_audio_clock_presents_seek_preview_without_waiting_for_wall_clock() {
        let directory = scratch("fvid-audio-clock-seek", &[]);
        let path = directory.join("clip.mp4");
        let fixture = include_bytes!("../../../tests/fixtures/audio/two-audio.mp4");
        std::fs::write(&path, fixture).unwrap();
        let mut player = Player {
            queue: vec![path],
            no_audio: true,
            ..Default::default()
        };
        player.play_index(0);
        let stream = crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(fixture.to_vec()),
            crate::container::mp4::Limits::default(),
        )
        .unwrap();
        player.audio = Some(crate::audio_thread::AudioPlayback::start(
            Box::new(stream),
            || Box::new(crate::audio::NullBackend::default()),
        ));
        player.seek_time(Duration::from_millis(500));
        player.buffering = false;
        player.deadline = Instant::now() + Duration::from_secs(10);
        let until = Instant::now() + Duration::from_secs(2);
        let ctx = eframe::egui::Context::default();
        while player.seek_preview {
            assert!(
                Instant::now() < until,
                "wall-clock gate blocked an audio-ready frame"
            );
            let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.presented.is_some());
        assert!(!player.audio_ended);
        assert!(!player.paused);
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ready_seek_picture_does_not_wait_for_future_spool_lead_after_audio_failure() {
        struct BrokenAudio;
        impl crate::audio::AudioStream for BrokenAudio {
            fn codec(&self) -> &str {
                "mp4a"
            }
            fn timescale(&self) -> u32 {
                44100
            }
            fn sample_rate(&self) -> u32 {
                44100
            }
            fn channels(&self) -> u16 {
                2
            }
            fn extra_data(&self) -> &[u8] {
                &[]
            }
            fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
                Vec::new()
            }
            fn next_packet(&mut self) -> crate::Result<Option<crate::audio::EncodedPacket>> {
                Ok(None)
            }
            fn rewind(&mut self) {}
            fn seek_to(&mut self, pts: i64) -> i64 {
                pts
            }
        }
        let directory = scratch("fvid-ready-seek-spool", &[]);
        let path = directory.join("clip.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/playback-errors/seek-ready-pending-spool.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            no_audio: true,
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.duration, Some(Duration::from_millis(320)));
        player.audio = Some(crate::audio_thread::AudioPlayback::start(
            Box::new(BrokenAudio),
            || Box::new(crate::audio::NullBackend::default()),
        ));
        let until = Instant::now() + Duration::from_secs(2);
        while !player.audio_ended {
            assert!(Instant::now() < until, "audio failure missing");
            player.poll_audio();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.audio.is_none());
        player.spool = Some(player.spool.as_ref().unwrap().pending_lead_for_test());
        player.spool_wait = Some(Instant::now());
        assert!(!player.spool_primed());
        let ctx = eframe::egui::Context::default();
        for target in [Duration::from_millis(120), Duration::from_millis(200)] {
            player.seek_time(target);
            let generation = player.playback.as_ref().unwrap().generation();
            let until = Instant::now() + Duration::from_secs(2);
            while player.seek_preview {
                assert!(
                    Instant::now() < until,
                    "ready seek picture held by future spool lead: queued={:?}",
                    player.queued.as_ref().map(|f| (f.generation, f.pts))
                );
                let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                !player.spool_primed(),
                "future copy must remain pending throughout this witness"
            );
            assert!(!player.buffering);
            assert!(!player.paused);
            assert!(!player.ended);
            assert_eq!(player.presented.as_ref().unwrap().generation, generation);
            assert!(player.timeline().0.unwrap() >= target);
        }
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn seeking_after_audio_failure_keeps_video_playing() {
        struct BrokenAudio;
        impl crate::audio::AudioStream for BrokenAudio {
            fn codec(&self) -> &str {
                "mp4a"
            }
            fn timescale(&self) -> u32 {
                44100
            }
            fn sample_rate(&self) -> u32 {
                44100
            }
            fn channels(&self) -> u16 {
                2
            }
            fn extra_data(&self) -> &[u8] {
                &[]
            }
            fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
                Vec::new()
            }
            fn next_packet(&mut self) -> crate::Result<Option<crate::audio::EncodedPacket>> {
                Ok(None)
            }
            fn rewind(&mut self) {}
            fn seek_to(&mut self, pts: i64) -> i64 {
                pts
            }
        }
        let directory = scratch("fvid-seek-failed-audio", &[]);
        let path = directory.join("clip.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            no_audio: true,
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        player.audio = Some(crate::audio_thread::AudioPlayback::start(
            Box::new(BrokenAudio),
            || Box::new(crate::audio::NullBackend::default()),
        ));
        let until = Instant::now() + Duration::from_secs(5);
        while !player.audio_ended {
            assert!(Instant::now() < until, "audio failure was not delivered");
            player.poll_audio();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            player.audio.is_none(),
            "failed worker must not survive to gate the next seek"
        );
        let ctx = eframe::egui::Context::default();
        for target in [Duration::from_millis(500), Duration::from_millis(1000)] {
            player.seek_time(target);
            let generation = player.playback.as_ref().unwrap().generation();
            let until = Instant::now() + Duration::from_secs(5);
            while player.seek_preview {
                assert!(Instant::now() < until, "seek stalled after audio failure: target={target:?} generation={generation} queued={:?} buffering={} paused={} ended={} audio={} audio_ended={} deadline_delta={:?} spool_primed={}",player.queued.as_ref().map(|f|(f.generation,f.pts,f.period)),player.buffering,player.paused,player.ended,player.audio.is_some(),player.audio_ended,Instant::now().checked_duration_since(player.deadline),player.spool_primed());
                assert!(player.error.is_none(), "{:?}", player.error);
                let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!player.paused);
            assert!(!player.ended);
            assert_eq!(player.presented.as_ref().unwrap().generation, generation);
            assert!(player.timeline().0.unwrap() >= target);
            while player.timeline().0.unwrap() < target + Duration::from_millis(150) {
                assert!(Instant::now() < until, "video froze after the seek preview");
                let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        drop(player);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn space_pauses_without_activating_the_focused_skin_button() {
        use eframe::egui::{self, Event, Key, Modifiers, RawInput, Sense};
        let ctx = egui::Context::default();
        let button = |ui: &mut egui::Ui| {
            ui.interact(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(100.0)),
                ui.id().with("skin"),
                Sense::click(),
            )
        };
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            button(ui).request_focus();
        });
        output.textures_delta.clear();
        let input = |key| RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input(Key::Space), |ui| {
            assert!(super::play_pause_pressed(ui.ctx()));
            assert!(!button(ui).clicked(), "Space must not switch the skin");
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(input(Key::Enter), |ui| {
            assert!(!super::play_pause_pressed(ui.ctx()));
            assert!(
                button(ui).clicked(),
                "Enter still activates the focused skin button"
            );
        });
        output.textures_delta.clear();
    }

    #[test]
    fn skin_options_reach_the_player() {
        assert!(play_args(&["--skin=diagram"]).unwrap().skin.diagram);
        assert!(!play_args(&["--skin", "classic"]).unwrap().skin.diagram);
        assert!(play_args(&["--skin"]).is_err());
        assert!(!Player::default().skin.diagram);
        assert!(!play_args(&[]).unwrap().skin.diagram);
    }

    /// The loaded span is a share of the item, so it is the same length on
    /// screen whatever the frame rate, and it never runs past the end or back
    /// before the picture.
    #[test]
    fn the_buffer_span_scales_with_the_item() {
        let tenth = Duration::from_millis(40);
        // Ten pictures of a 40 ms period are 0.4 s of picture: a thirtieth of a
        // twelve-second item, a tenth of a four-second one.
        assert!(
            (buffered_fraction(0.0, 10, tenth, Some(seconds(12_000))) - 1.0 / 30.0).abs() < 1e-6
        );
        assert!(
            (buffered_fraction(0.0, 10, tenth, Some(seconds(4_000))) - 1.0 / 10.0).abs() < 1e-6
        );
        // Further along the item, the same load adds the same share.
        assert!(
            (buffered_fraction(0.9, 10, tenth, Some(seconds(12_000))) - (0.9 + 1.0 / 30.0)).abs()
                < 1e-6
        );
        // And the span never runs past the end of the line.
        assert_eq!(
            buffered_fraction(0.99, 1_000, Duration::from_secs(1), Some(seconds(60_000))),
            1.0
        );
        // Nothing loaded reads as the picture itself, and an item of no stated
        // length has no share to draw.
        assert_eq!(
            buffered_fraction(0.25, 0, tenth, Some(seconds(12_000))),
            0.25
        );
        assert_eq!(buffered_fraction(0.25, 9, tenth, None), 0.25);
        assert_eq!(
            buffered_fraction(0.25, 9, tenth, Some(Duration::ZERO)),
            0.25
        );
    }

    /// The panel says the same thing in seconds, which is what differs between
    /// a full queue on a fast source and a full queue on a slow one.
    #[test]
    fn the_buffer_line_names_seconds_and_slots() {
        assert_eq!(
            buffer_text(6, 12, Duration::from_millis(40)),
            "Buffer 0.2 s · 6/12"
        );
        assert_eq!(
            buffer_text(0, 2, Duration::from_millis(40)),
            "Buffer 0.0 s · 0/2"
        );
    }

    /// A copy of a slow source is worth the picture it covers, so the panel
    /// says both how far it has got and how much of the file that is.
    #[test]
    fn the_local_copy_says_what_it_covers() {
        assert_eq!(
            spool_text(600, 1200, Duration::from_secs(60)),
            "Spool 30.0 s ahead"
        );
        assert_eq!(
            spool_text(0, 1200, Duration::from_secs(60)),
            "Spool 0.0 s ahead",
            "a bitten lead is the wait, not an empty copy"
        );
        // A window is a span of bytes, not a share of the file: a 3.6 GB item
        // at a 256 MiB lead can never show more than the lead is worth.
        assert_eq!(
            spool_text(256 << 20, 3603 << 20, Duration::from_secs(3603)),
            "Spool 256.0 s ahead"
        );
        assert_eq!(spool_text(0, 0, Duration::ZERO), "Spool 0.0 s ahead");
    }

    /// The swap on the line is a stretch of the item with two moving ends, not
    /// a share of it earned: a window in the middle of a file covers the
    /// middle, and a file with no stated size has no share for it to cover.
    #[test]
    fn the_local_copy_is_drawn_where_the_item_keeps_it() {
        assert_eq!(covered_span((0, 600), 1200), [0.0, 0.5]);
        assert_eq!(covered_span((600, 900), 1200), [0.5, 0.75]);
        // Past either end of the item the line has no more room for the copy.
        assert_eq!(
            covered_span((1000, 4000), 1200),
            [1000.0 / 1200.0, 1.0],
            "the far edge did not stop at the end of the item"
        );
        // A window the reader has run past reads as behind it rather than as a
        // span, which is what the drawing tests for before it paints anything.
        let behind = covered_span((900, 300), 1200);
        assert!(behind[0] > behind[1], "{behind:?} read as forwards");
        assert_eq!(covered_span((0, 10), 0), [0.0, 0.0]);
    }

    /// The preroll is a span of picture paid for in bytes, so it needs the
    /// item's size and length together; without one of them nothing is waited
    /// for, and a first picture is not held behind a question no one can answer.
    #[test]
    fn the_preroll_is_measured_against_the_item_it_buffers_for() {
        let player = Player {
            bytes: Some(1200),
            duration: Some(Duration::from_secs(60)),
            ..Default::default()
        };
        assert_eq!(player.spool_lead(), Some(120), "a tenth of the item");
        assert!(
            player.spool_primed(),
            "a source read directly has nothing to wait for"
        );
        let blind = Player {
            bytes: Some(1200),
            ..Default::default()
        };
        assert_eq!(blind.spool_lead(), None, "an unsized item has no lead");
        assert!(blind.spool_primed());
    }

    #[test]
    fn the_stop_time_ends_the_item_it_was_given_for() {
        let directory = scratch("fvid-player-stop-time", &[]);
        let path = directory.join("clip.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            bounds: PlayBounds {
                start: Some(seconds(500)),
                stop: Some(seconds(1_500)),
            },
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        // The start bound is spent on this open, the stop bound waits for the picture.
        assert_eq!(player.bounds, PlayBounds::default());
        assert_eq!(player.stop_time, Some(seconds(1_500)));
        assert_eq!(player.audio.as_ref().unwrap().generation(), 1,
            "the initial audio cursor must follow the video's start bound");

        // Before the picture has a position there is nothing to compare to.
        player.check_stop();
        assert!(!player.ended);

        player.seek_time(seconds(1_600));
        player.check_stop();
        assert!(player.ended, "a queue of one stops where it was told to");
        assert_eq!(player.stop_time, None, "the bound is spent either way");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_empty_list_has_nothing_to_continue_into() {
        let mut player = Player::default();
        player.continue_queue();
        assert!(player.ended);
    }

    /// VLC's loop key presses A, then B, then clears the pair. A press before
    /// the mark already set cannot loop backwards, so it opens a new region.
    #[test]
    fn the_loop_key_marks_a_then_b_then_clears() {
        assert_eq!(
            loop_press(seconds(2_000), None, None),
            LoopMark::MarkedA(seconds(2_000))
        );
        assert_eq!(
            loop_press(seconds(5_000), Some(seconds(2_000)), None),
            LoopMark::MarkedB(seconds(5_000))
        );
        assert_eq!(
            loop_press(seconds(9_000), Some(seconds(2_000)), Some(seconds(5_000))),
            LoopMark::Cleared
        );
        assert_eq!(
            loop_press(seconds(1_000), Some(seconds(2_000)), None),
            LoopMark::MarkedA(seconds(1_000))
        );
    }

    /// Only a closed region rewinds, and only once the picture has reached B.
    #[test]
    fn a_closed_region_rewinds_at_its_end() {
        let (a, b) = (seconds(2_000), seconds(5_000));
        assert_eq!(loop_rewind(seconds(4_999), Some(a), Some(b)), None);
        assert_eq!(loop_rewind(seconds(5_000), Some(a), Some(b)), Some(a));
        assert_eq!(loop_rewind(seconds(9_000), Some(a), None), None);
        assert_eq!(loop_rewind(seconds(9_000), None, Some(b)), None);
    }

    /// Rewinding is a seek, so a track without a sample index cannot loop, and
    /// the marks of the previous item do not carry into the next one.
    #[test]
    fn only_a_seekable_track_loops() {
        let directory = scratch("fvid-player-loop", &[]);
        std::fs::write(directory.join("clip.y4m"), y4m(2)).unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.y4m")],
            loop_a: Some(seconds(2_000)),
            loop_b: Some(seconds(5_000)),
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!((player.loop_a, player.loop_b), (None, None));
        assert!(!player.seekable);
        player.apply(Control::Loop);
        assert_eq!(player.osd.as_ref().unwrap().0, "Loop needs seeking");
        assert_eq!((player.loop_a, player.loop_b), (None, None));
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A positive delay is what a late voice track needs: the gate sees the
    /// audio clock as further ahead, so the picture runs on by that much.
    #[test]
    fn a_positive_audio_delay_lets_the_picture_run_ahead() {
        assert_eq!(delayed_clock(seconds(1_000), 0), seconds(1_000));
        assert_eq!(delayed_clock(seconds(1_000), 250), seconds(1_250));
        assert_eq!(delayed_clock(seconds(1_000), -250), seconds(750));
        // The clock cannot go before the start of the file.
        assert_eq!(delayed_clock(seconds(100), -250), Duration::ZERO);
    }

    /// The track key wraps within the tracks the container actually offered.
    #[test]
    fn audio_tracks_wrap_around_the_list() {
        assert_eq!(track_step(0, 3, true), 1);
        assert_eq!(track_step(2, 3, true), 0);
        assert_eq!(track_step(0, 3, false), 2);
        assert_eq!(track_step(1, 2, false), 0);
    }

    /// With nothing to choose between, the key says so rather than restarting
    /// the sound that is already playing.
    #[test]
    fn the_track_key_needs_a_choice_to_do_anything() {
        let mut player = Player::default();
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No audio");
        player.audio_tracks = vec![crate::audio::AudioTrack {
            sample_rate: 48_000,
            channels: 2,
            name: String::new(),
            language: String::new(),
        }];
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 1/1");
        assert!(player.audio.is_none());
    }

    #[test]
    fn the_audio_delay_key_is_bounded_at_ten_seconds() {
        let mut player = Player::default();
        player.apply(Control::AudioDelay(true));
        assert_eq!(player.audio_delay_ms, 50);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio delay 50 ms");
        for _ in 0..300 {
            player.apply(Control::AudioDelay(true));
        }
        assert_eq!(player.audio_delay_ms, 10_000);
        for _ in 0..500 {
            player.apply(Control::AudioDelay(false));
        }
        assert_eq!(player.audio_delay_ms, -10_000);
    }

    /// The chapter keys answer with the starts the list carries: the first one
    /// ahead going forward, the last one behind going back, and nothing to walk
    /// once the position stands at either end of the film.
    #[test]
    fn the_chapter_keys_walk_the_list_and_stop_at_either_end() {
        let starts = [seconds(0), seconds(1_000), seconds(3_000)];
        assert_eq!(chapter_ahead(&starts, seconds(0), true), Some(1));
        assert_eq!(chapter_ahead(&starts, seconds(500), true), Some(1));
        assert_eq!(chapter_ahead(&starts, seconds(1_000), true), Some(2));
        assert_eq!(chapter_ahead(&starts, seconds(3_000), true), None);
        assert_eq!(chapter_ahead(&starts, seconds(3_500), false), Some(2));
        assert_eq!(chapter_ahead(&starts, seconds(1_000), false), Some(0));
        assert_eq!(chapter_ahead(&starts, seconds(0), false), None);
    }

    /// The Matroska chapter atoms and the `chpl` of the MP4 muxed from the same
    /// film hand the player one and the same list; a file that names no parts,
    /// or one there is no reader for, leaves nothing to walk.
    #[test]
    fn both_containers_hand_the_player_the_same_chapter_list() {
        let directory = scratch("fvid-player-chapters", &[]);
        let matroska = directory.join("chapters.mkv");
        let movie = directory.join("chapters.mp4");
        std::fs::write(
            &matroska,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        std::fs::write(
            &movie,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mp4"),
        )
        .unwrap();
        let expected = [
            (Duration::ZERO, "Opening".to_string()),
            (Duration::from_secs(1), "Глава 2".to_string()),
            (Duration::from_secs(3), "End".to_string()),
        ];
        let shown = |marks: &[ChapterMark]| -> Vec<(Duration, String)> {
            marks
                .iter()
                .map(|mark| (mark.start, mark.title.clone()))
                .collect()
        };
        assert_eq!(shown(&container_facts(&matroska).chapters), expected);
        assert_eq!(shown(&container_facts(&movie).chapters), expected);
        assert!(
            container_facts(&directory.join("no-such-file.mkv"))
                .chapters
                .is_empty()
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file that names its parts: the key moves the picture to the start of
    /// the next one and says which, the end of the list says so instead of
    /// seeking, and the back key returns to the part just left.
    #[test]
    fn a_file_that_names_its_parts_jumps_between_them() {
        let directory = scratch("fvid-player-chapter-walk", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.seekable);
        assert_eq!(player.chapters.len(), 3);
        // The walk starts from where the picture stands, so put the first frame
        // of the film on the screen: a quarter of a second on the nanosecond
        // clock Matroska counts in.
        player.interval = Some((0, 250_000_000, 1_000_000_000));

        player.apply(Control::Chapter(true));
        assert_eq!(player.seek_target, Some(Duration::from_secs(1)));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Chapter 2/3 Глава 2",
            "{:?}",
            player.error
        );

        player.apply(Control::Chapter(true));
        assert_eq!(player.seek_target, Some(Duration::from_secs(3)));
        assert_eq!(player.osd.as_ref().unwrap().0, "Chapter 3/3 End");

        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Last chapter");
        assert_eq!(player.seek_target, Some(Duration::from_secs(3)));

        player.apply(Control::Chapter(false));
        assert_eq!(player.seek_target, Some(Duration::from_secs(1)));
        assert_eq!(player.osd.as_ref().unwrap().0, "Chapter 2/3 Глава 2");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// With no list in the file there is nothing to walk, and the key says so
    /// rather than restarting the position it cannot move.
    #[test]
    fn the_chapter_keys_say_when_the_file_names_no_parts() {
        let mut player = Player::default();
        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No chapters");
        player.chapters = vec![ChapterMark {
            start: Duration::from_secs(1),
            title: String::new(),
        }];
        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Cannot seek");
    }

    /// The switches and the two track numbers, read in either value form. A
    /// track is counted from one, because that is how the player's own messages
    /// count them, and zero is not a track anywhere.
    #[test]
    fn the_start_switches_and_the_track_numbers_count_from_one() {
        let parsed = play_args(&[
            "a.mp4",
            "--start-paused",
            "--no-audio",
            "--audio-track",
            "2",
            "--subtitle-track=3",
        ])
        .expect("the switches and both numbers");
        assert!(parsed.start_paused);
        assert!(parsed.no_audio);
        assert_eq!(
            (parsed.audio_track, parsed.subtitle_track),
            (Some(1), Some(2))
        );
        assert_eq!(parsed.paths.len(), 1);

        let plain = play_args(&["a.mp4"]).expect("no options at all");
        assert_eq!(
            (
                plain.start_paused,
                plain.no_audio,
                plain.audio_track,
                plain.subtitle_track
            ),
            (false, false, None, None)
        );
        assert!(
            play_args(&["--audio-track", "0"]).is_err(),
            "zero names no track"
        );
        assert!(play_args(&["--audio-track", "b"]).is_err());
        assert!(play_args(&["--subtitle-track"]).is_err());
        assert!(
            play_args(&["--start-paused=now"]).is_err(),
            "a switch takes no value"
        );
        assert!(play_args(&["--no-audio=yes"]).is_err());
    }

    /// `--start-paused` leaves the first picture on the screen and the item
    /// waiting, so the window is not a blank one; the play key lets it go.
    #[test]
    fn a_paused_start_shows_its_first_picture_and_waits() {
        let directory = scratch("fvid-player-start-paused", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path.clone()],
            start_paused: true,
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.paused);
        assert_eq!(player.step, 1, "one picture is asked for");
        assert!(player.buffering);
        player.toggle_pause();
        assert!(!player.paused);

        let mut running = Player {
            queue: vec![path],
            ..Default::default()
        };
        running.play_index(0);
        assert!(!running.paused);
        assert_eq!(running.step, 0);
        drop(player);
        drop(running);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The sound of the file, or none of it: `--no-audio` leaves the track list
    /// empty, and a track number picks the one the container offers. A number
    /// past the end of the list leaves the track there is to hear.
    #[test]
    fn the_command_line_can_pick_the_sound_or_leave_it_out() {
        let directory = scratch("fvid-player-audio-choice", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();

        let mut silent = Player {
            queue: vec![path.clone()],
            no_audio: true,
            ..Default::default()
        };
        silent.play_index(0);
        assert!(silent.error.is_none(), "{:?}", silent.error);
        assert!(silent.audio.is_none());
        assert!(silent.audio_tracks.is_empty());
        assert_eq!(silent.name, "two-audio.mp4");

        let mut second = Player {
            queue: vec![path.clone()],
            preferred_audio: Some(1),
            ..Default::default()
        };
        second.play_index(0);
        assert_eq!((second.audio_tracks.len(), second.audio_track), (2, 1));
        assert!(second.audio.is_some());

        let mut past_the_end = Player {
            queue: vec![path],
            preferred_audio: Some(9),
            ..Default::default()
        };
        past_the_end.play_index(0);
        assert_eq!(
            past_the_end.audio_track, 0,
            "a track there is not is left out"
        );
        drop(silent);
        drop(second);
        drop(past_the_end);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file with nothing to show is played by its sound, so a session that
    /// refuses sound has nothing left to open. The reason says which option said
    /// so, rather than repeating the picture's complaint.
    #[test]
    fn a_file_with_nothing_to_show_and_no_sound_allowed_refuses() {
        let directory = scratch("fvid-player-no-audio-sound-only", &[]);
        let path = directory.join("flac-stereo.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv"),
        )
        .unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.audio.is_some());
        assert!(heard.playback.is_none());

        let mut refused = Player {
            queue: vec![path],
            no_audio: true,
            ..Default::default()
        };
        refused.play_index(0);
        let message = refused.error.take().expect("the refusal is reported");
        assert!(
            message.contains("--no-audio"),
            "the refusal names the option: {message}"
        );
        drop(heard);
        drop(refused);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A Dolby Digital file on its own is sound the player can hear: the frame
    /// walk in [`crate::playback_ac3`] lists its packets, so the item opens as
    /// sound with the panel naming the coding the frames state. The surround
    /// fixture is a 5.1 stream, and the count the panel shows is the frame's own
    /// native layout rather than a container's claim.
    #[test]
    fn a_bare_dolby_digital_file_opens_as_sound_with_its_own_geometry() {
        let directory = scratch("fvid-player-elementary-ac3", &[]);
        let path = directory.join("ac3-51.ac3");
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/audio/ac3-51.ac3")).unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.playback.is_none(), "nothing to show");
        assert!(heard.audio.is_some(), "the sound carries the item");
        assert_eq!(heard.sound_codec, "Dolby Digital");
        assert_eq!(
            heard.audio_tracks.len(),
            1,
            "an elementary stream is one track"
        );
        assert_eq!(
            (
                heard.audio_tracks[0].sample_rate,
                heard.audio_tracks[0].channels
            ),
            (48_000, 6)
        );
        // Eight coded frames minus the decoder's 256-sample priming.
        // The player reports audible time, not encoded-frame duration.
        assert_eq!(heard.duration, Some(Duration::from_nanos((8*1536-256)*1_000_000_000/48000)));
        assert!(heard.seekable, "a walked stream has a seek table");
        drop(heard);

        // A track key past the one the file holds leaves it playing: the list the
        // stream itself reported is the bound, and the first track is the one there
        // is to hear.
        let mut asked_for_two = Player {
            queue: vec![path],
            preferred_audio: Some(1),
            ..Default::default()
        };
        asked_for_two.play_index(0);
        assert!(asked_for_two.error.is_none(), "{:?}", asked_for_two.error);
        assert_eq!(asked_for_two.audio_track, 0);
        assert_eq!(asked_for_two.sound_codec, "Dolby Digital");
        drop(asked_for_two);

        // The rate a stream's own frames state is the rate the item runs on: this
        // fixture is mono at 32 kHz over six frames, and no option says so.
        let mono = directory.join("ac3-mono-32k.ac3");
        std::fs::write(
            &mono,
            include_bytes!("../../../tests/fixtures/audio/ac3-mono-32k.ac3"),
        )
        .unwrap();
        let mut low = Player {
            queue: vec![mono],
            ..Default::default()
        };
        low.play_index(0);
        assert!(low.error.is_none(), "{:?}", low.error);
        assert_eq!(
            (
                low.audio_tracks[0].sample_rate,
                low.audio_tracks[0].channels
            ),
            (32_000, 1)
        );
        assert_eq!(low.duration, Some(Duration::from_nanos((6*1536-256)*1_000_000_000/32000)));
        drop(low);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An ADTS file on its own is sound the same way, with its geometry read out
    /// of the frames rather than a sample entry: the fixture is AAC-LC at 48 kHz
    /// over thirteen frames of 1024 samples. The panel names the coding the frames
    /// state even though this reader has to write the decoder's setup block
    /// itself, which is the one piece of an MP4 sample entry an elementary file
    /// holds nowhere.
    #[test]
    fn a_bare_adts_file_opens_as_sound_with_the_config_its_frames_state() {
        let directory = scratch("fvid-player-elementary-aac", &[]);
        let path = directory.join("aac-stereo.aac");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/audio/aac-stereo.aac"),
        )
        .unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.playback.is_none(), "nothing to show");
        assert!(heard.audio.is_some(), "the sound carries the item");
        assert_eq!(heard.sound_codec, "AAC");
        assert_eq!(
            heard.audio_tracks.len(),
            1,
            "an elementary stream is one track"
        );
        assert_eq!(
            (
                heard.audio_tracks[0].sample_rate,
                heard.audio_tracks[0].channels
            ),
            (48_000, 2)
        );
        // Thirteen frames of 1024 samples at 48 kHz: twelve of audio plus the
        // encoder's tag frame, which the reference demuxer counts as a packet too.
        assert_eq!(heard.duration.map(|at| at.as_micros()), Some(277_333));
        assert!(heard.seekable, "a walked stream has a seek table");
        drop(heard);

        // A second track key has nothing to land on, as with the other elementary
        // readers, and the one track the frames describe keeps playing.
        let mut asked_for_two = Player {
            queue: vec![path],
            preferred_audio: Some(1),
            ..Default::default()
        };
        asked_for_two.play_index(0);
        assert!(asked_for_two.error.is_none(), "{:?}", asked_for_two.error);
        assert_eq!(asked_for_two.audio_track, 0);
        assert_eq!(asked_for_two.sound_codec, "AAC");
        drop(asked_for_two);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The text the command line asked for is on screen from the first frame,
    /// and a source the file does not have leaves the one it does.
    #[test]
    fn the_subtitle_option_lands_on_the_track_it_names() {
        let directory = scratch("fvid-player-subtitle-choice", &[]);
        let path = directory.join("text-tracks.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();

        let mut second = Player {
            queue: vec![path.clone()],
            preferred_subtitle: Some(1),
            ..Default::default()
        };
        second.play_index(0);
        assert_eq!(
            (second.subtitle_sources.len(), second.subtitle_source),
            (2, 1)
        );
        assert_eq!(second.subtitle_line(seconds(700)), Some("other track"));

        let mut first = Player {
            queue: vec![path.clone()],
            preferred_subtitle: Some(9),
            ..Default::default()
        };
        first.play_index(0);
        assert_eq!(first.subtitle_source, 0);

        let mut none = Player {
            queue: vec![path],
            ..Default::default()
        };
        none.play_index(0);
        assert_eq!(none.subtitle_source, 0);
        drop(second);
        drop(first);
        drop(none);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The level, the mute switch, what the queue does at its end and both
    /// clock shifts, read in the units the player's own messages use: VLC's
    /// percent for the level and milliseconds for the delays, which the delay
    /// keys hold to ±10 s and so are held here too.
    #[test]
    fn the_level_the_repeat_and_both_delays_start_where_the_keys_hold_them() {
        let parsed = play_args(&[
            "a.mp4",
            "--volume",
            "150",
            "--mute",
            "--loop",
            "--random",
            "--audio-delay=-250",
            "--subtitle-delay",
            "12000",
            "--sub-file",
            "subs.srt",
        ])
        .expect("every start option");
        assert_eq!(parsed.volume_milli, 1_500);
        assert!(parsed.muted);
        assert_eq!(parsed.repeat, Repeat::All);
        assert!(parsed.shuffle, "a queue that walks a drawn order");
        assert_eq!(
            (parsed.audio_delay_ms, parsed.subtitle_delay_ms),
            (-250, 10_000)
        );
        assert_eq!(parsed.subtitle_file, Some(PathBuf::from("subs.srt")));

        let plain = play_args(&["a.mp4"]).expect("no options at all");
        assert_eq!(
            (
                plain.volume_milli,
                plain.muted,
                plain.repeat,
                plain.shuffle,
                plain.audio_delay_ms,
                plain.subtitle_delay_ms,
                plain.subtitle_file
            ),
            (1_000, false, Repeat::Off, false, 0, 0, None)
        );

        // The last of the two queue switches wins, the way the repeat key
        // replaces the mode it found rather than adding to it.
        assert_eq!(
            play_args(&["a.mp4", "--loop", "--repeat"])
                .expect("both queue switches")
                .repeat,
            Repeat::One
        );

        assert!(play_args(&["--volume", "201"]).is_err(), "past the slider");
        assert!(play_args(&["--volume", "b"]).is_err());
        assert!(play_args(&["--volume"]).is_err());
        assert!(
            play_args(&["--mute=yes"]).is_err(),
            "a switch takes no value"
        );
        assert!(play_args(&["--loop=1"]).is_err());
        assert!(play_args(&["--audio-delay", "x"]).is_err());
        assert!(play_args(&["--subtitle-delay"]).is_err());
        assert!(play_args(&["--sub-file"]).is_err());
    }

    /// A subtitle file the command line names leads the list the player guesses
    /// from the video's own name, and a name nothing can be read from says so
    /// instead of falling back unheard.
    #[test]
    fn a_named_subtitle_file_leads_the_list_the_player_guesses() {
        let directory = scratch("fvid-player-sub-file", &[]);
        let video = directory.join("film.mp4");
        std::fs::write(&video, b"not a container").unwrap();
        std::fs::write(
            directory.join("film.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nGuessed\n",
        )
        .unwrap();
        let named = directory.join("chosen.srt");
        std::fs::write(&named, "1\n00:00:01,000 --> 00:00:02,000\nNamed\n").unwrap();

        let mut player = Player {
            subtitle_file: Some(named.clone()),
            ..Default::default()
        };
        player.load_subtitles(&video);
        assert_eq!(
            player
                .subtitle_sources
                .iter()
                .map(|source| source.label.as_str())
                .collect::<Vec<_>>(),
            ["chosen.srt", "film.srt"]
        );
        assert_eq!(player.subtitle_name, "chosen.srt");
        assert_eq!(player.cues[0].text, "Named");

        player.subtitle_file = Some(directory.join("missing.srt"));
        player.load_subtitles(&video);
        assert_eq!(player.subtitle_sources[0].label, "film.srt");
        assert_eq!(player.subtitle_name, "film.srt");
        assert!(
            player
                .osd
                .as_ref()
                .unwrap()
                .0
                .starts_with("Cannot read subtitles"),
            "{:?}",
            player.osd
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A two-node cube that hands every code back: the look a test asks for when
    /// it wants a LUT file without wanting a colour change.
    const IDENTITY_CUBE: &str = "\
LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
";

    /// The three colour options name what the session grades with. A curve and a
    /// compression are matched to fvid's own spellings whatever the case, while a
    /// LUT has to be a file that actually parses: a word on the list, a readable
    /// grid.
    #[test]
    fn the_command_line_names_the_colour_grade_to_start_with() {
        let parsed = play_args(&["--log", "SLog3", "--tonemap=Hable", "c"]).unwrap();
        assert_eq!(parsed.grading.log, Some(Log::SLog3));
        assert_eq!(parsed.grading.tone_map, Some(ToneMap::Hable));
        assert_eq!(parsed.grading.lut, None);
        let parsed = play_args(&["--log", " logc4 ", "c"]).unwrap();
        assert_eq!(parsed.grading.log, Some(Log::LogC4));
        assert!(play_args(&["c"]).unwrap().grading.silent());
        for words in [
            vec!["--log", "slog4"],
            vec!["--log", "s-log3"],
            vec!["--log", ""],
            vec!["--log"],
            vec!["--tonemap", "mobius2"],
            vec!["--tonemap", "bt244"],
            vec!["--tonemap"],
            vec!["--lut", "no-such-file.cube"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a grade"
            );
        }
        // A refusal names what is on offer, so the caller sees the spellings
        // that would have worked.
        let error = play_args(&["--log", "slog4", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("slog3") && error.contains("logc4"),
            "{error}"
        );
        let error = play_args(&["--tonemap", "bt244", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("reinhard") && error.contains("mobius"),
            "{error}"
        );
    }

    /// `--gamut` names the working gamut the bytes are read in, which is how a
    /// container that labels them wrongly gets corrected: the flag outranks the
    /// primaries the file states and the triangle a `--log` curve lends, and it
    /// is the only route to the gamuts no curve lends — S-Gamut3.Cine, D-Gamut
    /// and F-Gamut C among them. Spellings fold case and separators, so
    /// `S-Gamut3.Cine`, `sgamut3_cine` and a vendor's own `AWG 4` all land on a
    /// triangle, and a word that names none stops the command at the door with
    /// the spellings that would have worked.
    #[test]
    fn the_command_line_names_the_gamut_the_bytes_are_read_in() {
        let parsed = play_args(&["--gamut", "S-Gamut3.Cine", "c"]).unwrap();
        assert_eq!(parsed.grading.gamut, Some(Primaries::S_GAMUT3_CINE));
        assert_eq!(
            play_args(&["--gamut=sgamut3_cine", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::S_GAMUT3_CINE)
        );
        assert_eq!(
            play_args(&["--gamut", " AWG 4 ", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::ALEX3_EXPANDED)
        );
        assert_eq!(
            play_args(&["--gamut", "rec709", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::BT709)
        );
        assert!(play_args(&["c"]).unwrap().grading.gamut.is_none());
        for words in [
            vec!["--gamut", "sgamut"],
            vec!["--gamut", "awg3"],
            vec!["--gamut", ""],
            vec!["--gamut"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} names no gamut"
            );
        }
        let error = play_args(&["--gamut", "sgamut", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("S-Gamut3.Cine") && error.contains("F-Gamut C"),
            "{error}"
        );
    }

    /// `--display` names the panel the session grades for, and a panel is a
    /// request in its own right: it moves where the picture's light lands
    /// whether the item states BT.2100 or nothing at all.
    #[test]
    fn a_named_panel_moves_where_the_picture_lands() {
        let empty = HdrMetadata::default();
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        // A brighter SDR panel has a higher white, so the same 100 cd/m² of the
        // master is a darker code on the screen that reaches 600.
        let panel = |nits| Grading {
            panel: Some(Panel::Sdr(nits)),
            ..Default::default()
        };
        let lid = Grading::default().grade_for(hdr10, &empty).unwrap();
        let bright = panel(600.0).grade_for(hdr10, &empty).unwrap();
        assert_eq!(lid.plan().target, Some(DisplayTarget::sdr(PANEL_NITS)));
        assert_eq!(bright.plan().target, Some(DisplayTarget::sdr(600.0)));
        assert!(lid.rgb([0.5; 3])[0] > 0.9, "on a 100-nit lid: {lid:?}");
        assert!(
            bright.rgb([0.5; 3])[0] < 0.45,
            "on a 600-nit panel: {:?}",
            bright.rgb([0.5; 3])
        );
        // A BT.2100 destination writes BT.2100 codes over BT.2020 and runs no
        // shoulder, which on HDR10 material is the whole point: the bytes the
        // file holds are the picture the screen wants.
        let hdr = Grading {
            panel: Some(Panel::Pq(1_000.0)),
            ..Default::default()
        }
        .grade_for(hdr10, &empty)
        .unwrap();
        let plan = hdr.plan();
        assert_eq!(
            (plan.to, plan.dest, plan.tone_map),
            (Transfer::Pq, Primaries::BT2020, None)
        );
        assert!(hdr.is_identity(), "HDR10 onto HDR10 changes nothing");
        // An SDR item asked for a PQ destination is the same picture in codes
        // that state its light: white leaves at the panel's diffuse white, not
        // at the top of PQ's 10 000-nit scale.
        let up = Grading {
            panel: Some(Panel::Pq(1_000.0)),
            ..Default::default()
        }
        .grade_for(video, &empty)
        .unwrap();
        assert!(!up.is_identity());
        let nits = Transfer::Pq.to_nits(up.rgb([1.0; 3])[0], 262.0).unwrap();
        assert!(
            (nits - 262.0).abs() < 6.0,
            "SDR white reached {nits} cd/m² on a 1 000-nit PQ panel"
        );
        // The peak a caller names is what an HLG code means, so two panels of
        // different capability cannot share the grade.
        let hlg = |peak| Grading {
            panel: Some(Panel::Hlg(peak)),
            ..Default::default()
        };
        let small = hlg(400.0).grade_for(hdr10, &empty).unwrap();
        let big = hlg(1_000.0).grade_for(hdr10, &empty).unwrap();
        assert_eq!(
            (small.plan().to, small.plan().dest),
            (Transfer::Hlg, Primaries::BT2020)
        );
        assert_eq!(small.plan().target, Some(DisplayTarget::hdr(400.0)));
        assert_ne!(small.rgb([0.5; 3]), big.rgb([0.5; 3]));
    }

    /// `--display` takes a kind and optionally the cd/m² it reaches, in either
    /// spelling of the option; anything else stops the command at the door.
    #[test]
    fn the_command_line_names_the_panel_the_picture_is_graded_for() {
        let parsed = |args: &[&str]| play_args(args).unwrap().grading.panel;
        assert_eq!(parsed(&["--display=pq", "c"]), Some(Panel::Pq(1_000.0)));
        assert_eq!(
            parsed(&["--display", "hdr:600", "c"]),
            Some(Panel::Pq(600.0))
        );
        assert_eq!(
            parsed(&["--display", " SDR : 240 ", "c"]),
            Some(Panel::Sdr(240.0))
        );
        assert_eq!(parsed(&["--display=hlg", "c"]), Some(Panel::Hlg(1_000.0)));
        assert_eq!(
            parsed(&["--display=sdr", "c"]),
            Some(Panel::Sdr(PANEL_NITS))
        );
        assert!(parsed(&["c"]).is_none());
        for bad in [
            "--display=bt2020",
            "--display=sdr:0",
            "--display=pq:99999",
            "--display=hlg:high",
            "--display=pq:",
        ] {
            let error = play_args(&[bad, "c"]).err().unwrap().to_string();
            assert!(error.contains("--display"), "{bad} said {error:?}");
        }
        // A kind fvid writes no codes for is answered with the kinds it does.
        let error = play_args(&["--display=bt2020", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("sdr[:nits]") && error.contains("hlg[:nits]"),
            "{error}"
        );
        // With no value at all the message is the one every option of this
        // command gives, which names the option rather than its grammar.
        assert!(
            play_args(&["--display", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("--display")
        );
    }

    /// Naming a gamut is a request in its own right, not a modifier: an SDR item
    /// that states BT.709 has to be re-read as the gamut the caller names, and
    /// written back in the signal the file states for itself. The picture is not
    /// the one the label implied, so nothing here is an identity.
    #[test]
    fn a_named_gamut_grades_an_item_that_asked_for_nothing_else() {
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let grade = Grading {
            gamut: Some(Primaries::S_GAMUT3_CINE),
            ..Default::default()
        }
        .grade_for(video, &HdrMetadata::default())
        .expect("a named gamut is a request");
        let plan = grade.plan();
        assert_eq!(plan.source, Primaries::S_GAMUT3_CINE);
        assert_eq!(
            (plan.from, plan.to, plan.dest),
            (Transfer::Bt709, Transfer::Bt709, Primaries::BT709)
        );
        assert!(!grade.is_identity());
        // With a curve, the name still wins over the triangle the profile lends.
        let graded = Grading {
            gamut: Some(Primaries::D_GAMUT),
            log: Some(Log::VLog),
            ..Default::default()
        }
        .grade_for(ColourDescription::default(), &HdrMetadata::default())
        .unwrap();
        assert_eq!(graded.plan().source, Primaries::D_GAMUT);
    }

    /// A cube's edge and the way its nodes are joined say how finely the colour
    /// decision is baked, so they move the picture a request produces; naming
    /// either of them alone asks for no picture at all.
    #[test]
    fn a_named_grid_and_reading_change_how_the_picture_is_baked() {
        let empty = HdrMetadata::default();
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let baked = |grid, interp| {
            Grading {
                grid,
                interp,
                ..Default::default()
            }
            .grade_for(hdr10, &empty)
            .unwrap()
        };
        assert_eq!(baked(None, None).plan().size, 33);
        let coarse = baked(Some(2), None);
        let fine = baked(Some(64), None);
        assert_eq!((coarse.plan().size, fine.plan().size), (2, 64));
        assert_ne!(coarse.rgb([0.5; 3]), fine.rgb([0.5; 3]));
        let stepped = baked(Some(2), Some(Interpolation::Nearest));
        assert_ne!(stepped.rgb([0.5; 3]), coarse.rgb([0.5; 3]));
        // The same reading is what a look from a file gets: a two-node grid whose
        // top corner is off-white blends towards it when the box around a point
        // is read, and keeps its step when only the nearest corner is.
        let look = Lut::from_cube(
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 0\n",
        )
        .unwrap();
        let with = |interp| {
            Grading {
                lut: Some(look.clone()),
                interp,
                ..Default::default()
            }
            .grade_for(video, &empty)
            .unwrap()
        };
        assert_ne!(
            with(None).rgb([0.5; 3]),
            with(Some(Interpolation::Nearest)).rgb([0.5; 3])
        );
        for modifier in [
            Grading {
                grid: Some(64),
                ..Default::default()
            },
            Grading {
                interp: Some(Interpolation::Nearest),
                ..Default::default()
            },
        ] {
            assert!(modifier.grade_for(video, &empty).is_none());
        }
    }

    /// `--grid` takes the node count the conversion is baked at and `--interp`
    /// one of the readings fvid knows, in either spelling of the option; the
    /// bound and the set of readings are both in the answer a mistake gets.
    #[test]
    fn the_command_line_names_the_cube_and_how_its_nodes_are_joined() {
        let grid = |args: &[&str]| play_args(args).unwrap().grading.grid;
        let interp = |args: &[&str]| play_args(args).unwrap().grading.interp;
        assert_eq!(grid(&["--grid=64", "c"]), Some(64));
        assert_eq!(grid(&["--grid", " 12 ", "c"]), Some(12));
        assert_eq!(grid(&["--grid", "2", "c"]), Some(2));
        assert_eq!(grid(&["--grid", "128", "c"]), Some(128));
        assert!(grid(&["c"]).is_none());
        assert_eq!(
            interp(&["--interp=nearest", "c"]),
            Some(Interpolation::Nearest)
        );
        assert_eq!(
            interp(&["--interp", " TRILINEAR ", "c"]),
            Some(Interpolation::Trilinear)
        );
        assert_eq!(
            interp(&["--interp=linear", "c"]),
            Some(Interpolation::Trilinear)
        );
        assert_eq!(
            interp(&["--interp", "tetrahedral", "c"]),
            Some(Interpolation::Tetrahedral)
        );
        assert!(interp(&["c"]).is_none());
        for bad in [
            "--grid=1",
            "--grid=129",
            "--grid=deep",
            "--grid=-4",
            "--interp=bilinear",
            "--interp=",
        ] {
            let option = if bad.starts_with("--grid") {
                "--grid"
            } else {
                "--interp"
            };
            let error = play_args(&[bad, "c"]).err().unwrap().to_string();
            assert!(error.contains(option), "{bad} said {error:?}");
        }
        assert!(
            play_args(&["--grid=129", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("2..=128")
        );
        let error = play_args(&["--interp=bilinear", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("nearest")
                && error.contains("trilinear")
                && error.contains("tetrahedral"),
            "{error}"
        );
        // With no value at all the message names the option, as every option of
        // this command does.
        assert!(
            play_args(&["--grid", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("--grid")
        );
    }

    /// The panel says the three things a screen cannot show on its own: what the
    /// item states its colour to be, what light it carries, and what the grade
    /// does with the two.
    #[test]
    fn the_panel_says_what_a_picture_is_and_what_is_done_to_it() {
        let bt709 = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        // The signal every screen expects is not worth a line of its own.
        assert!(colour_line(bt709).is_none());
        // Neither is a file that wrote no colour at all: three zeros are
        // H.273's way of saying unspecified, and calling matrix 0 "RGB" would
        // report a shape the file never claimed.
        assert!(colour_line(ColourDescription::default()).is_none());
        // The same triangle over the full range is, since the grade reads its
        // codes by a different rule.
        assert_eq!(
            colour_line(ColourDescription {
                full_range: true,
                ..bt709
            })
            .unwrap(),
            "Colour: BT.709 · BT.709 · BT.709 · full"
        );
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        assert_eq!(
            colour_line(hdr10).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · limited"
        );
        // A part the container leaves unstated is named as unstated, because it
        // is the part the grade has to answer for.
        assert_eq!(
            colour_line(ColourDescription {
                primaries: 2,
                transfer: 2,
                matrix: 2,
                full_range: false,
            })
            .unwrap(),
            "Colour: not stated · unknown · not stated · limited"
        );
        // A part the file does state, by a number this module cannot resolve, is
        // reported by that number: "not stated" would blame the file for saying
        // nothing when it said something fvid has no table for. The other two
        // halves of the same triple are named, because their numbers do land.
        assert_eq!(
            colour_line(ColourDescription {
                primaries: 13,
                transfer: 13,
                matrix: 13,
                full_range: true,
            })
            .unwrap(),
            "Colour: code 13 · sRGB · chroma-derived CL · full"
        );
        let display = MasteringDisplay::from_corners(
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
            1_000.0,
            0.005,
        )
        .unwrap();
        let hdr = HdrMetadata {
            mastering: Some(display),
            light: ContentLight {
                max_cll: 1_000.0,
                max_fall: 400.0,
            },
        };
        assert!(light_line(&HdrMetadata::default()).is_none());
        assert_eq!(
            light_line(&hdr).unwrap(),
            "HDR: BT.2020 · 0.005–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²"
        );
        // A limit the file does not state is left off the line rather than
        // invented for it.
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(display),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: BT.2020 · 0.005–1000 cd/m²"
        );
        // A volume no table rounds onto is another panel, and the line says so
        // instead of naming the nearest one it half matches.
        let projector = MasteringDisplay::from_corners(
            (0.680, 0.320),
            (0.265, 0.690),
            (0.150, 0.060),
            (0.314, 0.351),
            500.0,
            0.001,
        )
        .unwrap();
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(projector),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: DCI-P3 · 0.001–500 cd/m²"
        );
        let odd = MasteringDisplay::from_corners(
            (0.680, 0.320),
            (0.200, 0.750),
            (0.150, 0.060),
            (0.3127, 0.3290),
            4_000.0,
            0.0001,
        )
        .unwrap();
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(odd),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: custom · 0.0001–4000 cd/m²"
        );
        // The grade line tells what is being done, which is not only what was
        // asked: HDR10 on the panel every session starts with compresses its
        // highlights although no shoulder was named, and the one picked is on
        // the line along with the grid it is baked on.
        let grade = Grading::default().grade_for(hdr10, &hdr).unwrap();
        let text = grade_text(&grade.plan(), grade.interpolation(), None);
        assert!(
            text.contains("PQ → BT.709")
                && text.contains("BT.2020 → BT.709")
                && text.contains("33³ tetrahedral"),
            "{text}"
        );
        // A look the session was given is on the same line, by the shape and
        // node count it was read as: a file's title does not survive the parse,
        // and how coarse the grid under the picture is what a viewer weighs.
        let looked = Grading {
            lut: Some(Lut::from_cube(IDENTITY_CUBE).unwrap()),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let looked_text = grade_text(&looked.plan(), looked.interpolation(), looked.lut());
        assert!(looked_text.contains("look 2³"), "{looked_text}");
        let lifted = Grading {
            lut: Some(Lut::from_cube("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap()),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let lifted_text = grade_text(&lifted.plan(), lifted.interpolation(), lifted.lut());
        assert!(lifted_text.contains("look 1D 2"), "{lifted_text}");
        // A log curve named at the door is the first word of the line, because
        // the grade reads the bytes as that curve rather than as the curve the
        // file states — the thing a viewer blames when a picture comes out flat.
        // It is the same word the option takes, so the panel and the command
        // line say the curve one way.
        let logged = Grading {
            log: Some(Log::SLog3),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let logged_text = grade_text(&logged.plan(), logged.interpolation(), logged.lut());
        assert!(logged_text.starts_with("slog3 → BT.709"), "{logged_text}");
        // The sample file writes no colour element and asks for no grade, so the
        // panel shows only the picture's own line; the three lines are then put
        // on by hand to check the order the panel keeps them in.
        let directory = scratch("fvid-player-info-colour", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        // The absence on the panel comes from the file stating nothing, not from
        // the helper dropping a signal the reader carried.
        assert_eq!(player.signal, ColourDescription::default());
        assert!(player.hdr.is_empty());
        assert!(player.grade_line.is_none());
        let plain = player.info_lines();
        assert_eq!(plain[0], "Video: VP9 · 16×16 · 4 fps");
        assert!(
            !plain
                .iter()
                .any(|line| line.starts_with("Colour:") || line.starts_with("Grade:")),
            "{plain:?}"
        );
        player.signal = hdr10;
        player.hdr = hdr;
        player.grade_line = Some(format!("Grade: {text}"));
        let lines = player.info_lines();
        assert_eq!(
            lines[1..4],
            [
                "Colour: BT.2020 · PQ · BT.2020 NCL · limited",
                "HDR: BT.2020 · 0.005–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²",
                format!("Grade: {text}").as_str(),
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// `--lut` reads the look at startup, from a file named either way round:
    /// `.cube` and `.3dl` are told apart by their contents, not their suffix, and
    /// a file that holds neither stops the command before a window opens.
    #[test]
    fn the_command_line_reads_the_lut_before_anything_is_opened() {
        let directory = std::env::temp_dir().join("fvid-player-lut-option");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let cube = directory.join("identity.cube");
        std::fs::write(&cube, IDENTITY_CUBE).unwrap();
        let parsed = play_args(&["--lut", cube.to_str().unwrap(), "c"]).unwrap();
        assert_eq!(
            parsed.grading.lut,
            Some(Lut::from_cube(IDENTITY_CUBE).unwrap())
        );
        // The same bytes under the wrong name are still a cube.
        let mislabelled = directory.join("identity.3dl");
        std::fs::write(&mislabelled, IDENTITY_CUBE).unwrap();
        assert!(
            play_args(&["--lut", mislabelled.to_str().unwrap(), "c"])
                .unwrap()
                .grading
                .lut
                .is_some()
        );
        let broken = directory.join("broken.cube");
        std::fs::write(&broken, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        let error = play_args(&["--lut", broken.to_str().unwrap(), "c"])
            .err()
            .unwrap();
        // The file's own name is in the message, since that is what the caller
        // has to go and fix.
        assert!(error.to_string().contains("broken.cube"), "{error}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One request, several files: each item is graded from the signal it states
    /// for itself. BT.2100 material is compressed for the screen even when the
    /// session asked for nothing, a log file is read the way the camera recorded
    /// it, and a look over a stated curve is the look alone.
    #[test]
    fn each_item_is_graded_from_the_signal_it_states() {
        let empty = HdrMetadata::default();
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        // Nothing asked and nothing to undo: the bytes go to the screen as read.
        assert!(Grading::default().grade_for(video, &empty).is_none());
        assert!(
            Grading::default()
                .grade_for(ColourDescription::default(), &empty)
                .is_none()
        );
        // HDR10 states BT.2100 codes over BT.2020 primaries, which no desktop
        // panel shows unmapped, so the grade is on without being asked for.
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        let grade = Grading::default().grade_for(hdr10, &empty).unwrap();
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Pq, Transfer::Bt709));
        assert_eq!(
            (plan.source, plan.dest),
            (Primaries::BT2020, Primaries::BT709)
        );
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        // `--tonemap` replaces the curve the plan would have picked.
        let hable = Grading {
            tone_map: Some(ToneMap::Hable),
            ..Default::default()
        }
        .grade_for(hdr10, &empty)
        .unwrap();
        assert_eq!(hable.plan().tone_map, Some(ToneMap::Hable));
        // `--log` reads the codes as the camera wrote them and hands the panel
        // finished video instead; the file's own primaries still win over the
        // profile's working gamut, because they describe these bytes.
        let logged = Grading {
            log: Some(Log::VLog),
            ..Default::default()
        }
        .grade_for(video, &empty)
        .unwrap();
        let plan = logged.plan();
        assert_eq!(plan.log, Some(Log::VLog));
        assert_eq!((plan.from, plan.to), (Transfer::Linear, Transfer::Bt709));
        assert_eq!(plan.source, Primaries::BT709);
        assert!(plan.tone_map.is_none());
        // A look is asked for on its own, so the plan writes the item's own
        // signal back and the file's own curve becomes the destination. Nothing
        // in the grid moves: an identity look on any file is no change at all.
        let look = Grading {
            lut: Some(Lut::from_cube(IDENTITY_CUBE).unwrap()),
            ..Default::default()
        };
        let grade = look.grade_for(video, &empty).unwrap();
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Bt709, Transfer::Bt709));
        assert_eq!(plan.source, plan.dest);
        assert!(plan.tone_map.is_none());
        assert!(grade.is_identity());
        // A file that names no curve is read as the codes come out of the
        // converter, so a look over it is still the look alone.
        let grade = look
            .grade_for(ColourDescription::default(), &empty)
            .unwrap();
        assert_eq!(grade.plan().from, grade.plan().to);
        assert!(grade.is_identity());
    }

    /// The chain end to end on a real file: this clip's light exists only as
    /// in-band SEI messages, which the decoder reads out of the configuration
    /// record as it is built, so a session that was told nothing still grades the
    /// file against the 1 000 cd/m² peak the encoder stated rather than a blind
    /// guess. That peak is also what the default curve is chosen from: the panel
    /// reaches 100 cd/m² and the master was authored at ten times that, so the
    /// session rolls its highlights instead of folding them to white, which a
    /// grade that never read the peak cannot show.
    #[test]
    fn a_files_in_band_light_reaches_the_grade_the_session_bakes() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../../../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        assert_eq!(grade.plan().content.max_cll, 1_000.0);
        // Asked for by name, the same peak steers the curve that reads it.
        let reinhard = Grading {
            tone_map: Some(ToneMap::Reinhard),
            ..Default::default()
        }
        .grade_for(signal, &hdr)
        .unwrap();
        assert_eq!(reinhard.plan().content.max_cll, 1_000.0);
        let highlight = [1.0, 0.85, 0.7];
        let blind = Grading {
            tone_map: Some(ToneMap::Reinhard),
            ..Default::default()
        }
        .grade_for(signal, &HdrMetadata::default())
        .unwrap();
        assert_ne!(reinhard.rgb(highlight), blind.rgb(highlight));
        // The default reads the peak too, and that is the difference a viewer
        // sees: a code above the panel's own reach keeps its place in the
        // shoulder instead of folding into white.
        let blind_default = Grading::default()
            .grade_for(signal, &HdrMetadata::default())
            .unwrap();
        assert_eq!(grade.plan().tone_map, Some(ToneMap::Mobius));
        assert_eq!(blind_default.plan().tone_map, Some(ToneMap::Clip));
        // PQ's 0.62 is about 190 cd/m², which a 100-nit panel has no number for
        // until the master's own peak says how far above it to roll.
        let kept = grade.rgb([0.62; 3])[0];
        let burnt = blind_default.rgb([0.62; 3])[0];
        assert!(burnt > 0.999, "{burnt}");
        assert!((0.85..0.96).contains(&kept), "{kept}");
        // And a code the panel shows outright is untouched by the choice.
        assert_eq!(grade.rgb([0.3; 3]), blind_default.rgb([0.3; 3]));
        // The panel words the same decision, from the file's own bytes: the
        // corners are BT.2020 in the payload's multiples of 0.00002, which the
        // line names as a panel rather than leaving as an exact-compare miss.
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · limited"
        );
        assert_eq!(
            light_line(&hdr).unwrap(),
            "HDR: BT.2020 · 0.0001–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²"
        );
        assert_eq!(
            grade_text(&grade.plan(), grade.interpolation(), grade.lut()),
            "PQ → BT.709 · BT.2020 → BT.709 · mobius · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on an H.264 clip that states HLG in the only place the
    /// coding has for it: the VUI of the sequence parameter set inside the
    /// `avcC` record, with no `colr` atom in the file at all. The triple has to
    /// answer when the file opens, because that is when the grade is baked — a
    /// reader that waited for a picture to select its parameter set would show
    /// this clip as if its codes were BT.709.
    #[test]
    fn an_avc_records_vui_is_graded_at_open() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../../../tests/fixtures/avc/hlg-vui-only.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        assert!(hdr.is_empty());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Hlg, Transfer::Bt709));
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · HLG · BT.2020 NCL · limited"
        );
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "HLG → BT.709 · BT.2020 → BT.709 · clip · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on an AV1 file that writes its signal in no container
    /// element at all: SVT-AV1 stated the BT.2020 triple in its sequence header
    /// and the HDR10 volume and a 1 234/567 light in metadata OBUs inside the
    /// first packet, and the muxer's `Colour` element holds one range flag. The
    /// grade a session bakes before showing a picture still reads the encoder's
    /// own peak, because the reader unpacks that packet while the file opens.
    /// And the coding's range flag replaces the container's, since a lone range
    /// with no triple beside it states no signal for the composition to keep.
    #[test]
    fn an_av1_tracks_in_band_signal_is_graded_at_open() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../../../tests/fixtures/av1/hdr-in-band.mkv").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert_eq!(
            signal,
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            }
        );
        assert!(signal.is_hdr());
        assert_eq!(hdr.content_light(PANEL_NITS).max_cll, 1_234.0);
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Pq, Transfer::Bt709));
        assert_eq!(plan.tone_map, Some(ToneMap::Mobius));
        assert_eq!(plan.content.max_cll, 1_234.0);
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · full"
        );
        assert_eq!(
            light_line(&hdr).unwrap(),
            // The floor is AV1's own number, not ST 2086's: the volume writes
            // luminance in 18.14 fixed point, so the black this encoder means
            // arrives as 2/16 384 rather than the 0.0001 a `mdcv` box carries.
            // The name above is unaffected — the corners and the 1 000 cd/m²
            // peak decide it, and no floor is compared — and the line states
            // what the file says rather than rounding it toward the other
            // standard's quantum.
            "HDR: BT.2020 · 0.00012207031–1000 cd/m² · MaxCLL 1234 cd/m² · MaxFALL 567 cd/m²"
        );
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "PQ → BT.709 · BT.2020 → BT.709 · mobius · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on a file that states a curve and no light at all: an HLG
    /// clip. Nothing was asked from the command line, yet BT.2100 material still
    /// gets the display-domain pass [`Grade::new`] picks — and because the panel's
    /// own peak is the only peak this file states, the headroom rule answers
    /// "nothing to roll", which for HLG is not a compromise but what the format
    /// means, its scene light being normalised to whatever display shows it. So
    /// the top of the code scale stays separated on the way to the screen instead
    /// of folding into white, and mid-code grey arrives as grey.
    #[test]
    fn an_hlg_item_is_graded_at_the_panels_own_peak() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../../../tests/fixtures/hevc/hlg.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        assert!(hdr.is_empty());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!(plan.from, Transfer::Hlg);
        assert_eq!(plan.to, Transfer::Bt709);
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        assert_eq!(plan.content.max_cll, PANEL_NITS);
        // Four codes that a 1 000-cd/m² reading of this signal would have
        // crushed together into white stay apart, in order, with mid grey below
        // half the scale.
        let steps: Vec<f32> = [0.5f32, 0.65, 0.8, 1.0]
            .iter()
            .map(|c| grade.rgb([*c; 3])[0])
            .collect();
        assert!(steps[0] < 0.5, "{steps:?}");
        assert!(steps.windows(2).all(|w| w[1] > w[0] + 0.05), "{steps:?}");
        assert!((steps[3] - 1.0).abs() < 1e-3, "{steps:?}");
        // The panel's words for this file: HLG states no light, so only the
        // signal it does state is on the panel, and the grade line says which
        // display-domain pass it took and against what peak.
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · HLG · BT.2020 NCL · limited"
        );
        assert!(light_line(&hdr).is_none());
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "HLG → BT.709 · BT.2020 → BT.709 · clip · 100 cd/m² · 33³ tetrahedral"
        );
    }
}

#[cfg(test)]
mod gpu_option_tests {
    use super::*;
    #[test]
    fn rgb_presentation_keeps_codes_without_a_second_cpu_texture() {
        let path = std::env::temp_dir().join(format!("fvid-rgb-presentation-{}.y4m", std::process::id()));
        let mut bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C444\nFRAME\n".to_vec();
        bytes.extend_from_slice(&[128; 12]);
        std::fs::write(&path, bytes).unwrap();
        let mut player = Player { no_audio: true, ..Default::default() };
        player.open(path.clone()).unwrap();
        let codes = vec![30, 80, 200, 50, 90, 180, 70, 100, 160, 90, 110, 140];
        player.queued = Some(crate::playback_thread::Frame {
            pixels: Pixels::Rgb(codes.clone()), dimensions: [2,2],
            period: Duration::from_millis(40), interval: None, pts: None,
            generation: player.playback.as_ref().unwrap().generation(), serial: 100,
            rotation: 0,
        });
        player.step = 1;
        player.present(&egui::Context::default());
        assert!(player.texture.is_none());
        assert_eq!(player.rgb_frame.as_ref().unwrap().0.as_slice(), codes);
        assert!(player.picture().unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        drop(player);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn player_gpu_selection_is_explicit() {
        let args: Vec<_> = ["--backend=metal", "--device", "2", "clip.mp4"].into_iter().map(str::to_owned).collect();
        let parsed = parse_play_args(&args).unwrap();
        assert_eq!(parsed.gpu_backend, crate::Backend::Metal);
        assert_eq!(parsed.gpu_device, 2);
        for flags in [["--backend", "cuda"], ["--backend", "cpu"], ["--device", "1"]] {
            let args: Vec<_> = flags.into_iter().map(str::to_owned).collect();
            assert!(parse_play_args(&args).is_err());
        }
        let args: Vec<_> = ["--shader", concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/grayscale.wgsl"), "clip.mp4"].into_iter().map(str::to_owned).collect();
        assert!(parse_play_args(&args).unwrap().shader.is_some());
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical VideoToolbox decoding"]
    fn metal_player_presents_shared_main10_and_can_snapshot_it() {
        let mut player = Player { shared_surfaces: true, no_audio: true, start_paused: true, ..Default::default() };
        player.begin_open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4"));
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        while player.video_surface.is_none() {
            assert!(Instant::now() < deadline, "shared picture never reached presentation");
            player.poll_open(&ctx);
            player.present(&ctx);
            assert!(player.error.is_none(), "{:?}", player.error);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(player.video.is_none()); assert!(player.video_packed.is_none()); assert!(player.texture.is_none());
        assert!(player.hardware);
        let (surface, _) = player.video_surface.as_ref().unwrap();
        assert_eq!(surface.surface.depth(), 10);
        assert!(matches!(player.presented.as_ref().unwrap().pixels, Pixels::Surface(_)));
        assert!(player.picture().unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(player.paused);
        assert_eq!(player.step, 0);
        assert_eq!(player.presented.as_ref().unwrap().pts.map(|(pts, _)| pts), Some(0));
        let serial = player.presented.as_ref().unwrap().serial;
        for _ in 0..10 {
            player.present(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(player.presented.as_ref().unwrap().serial, serial);
    }

}

#[cfg(test)]
mod http_input_tests {
    use super::*;
    #[test]
    fn closing_download_owner_cancels_the_worker_flag() {
        let flag = fvid_media::CancelFlag::new();
        let owner = NetworkDownloadCancel(flag.clone());
        assert!(!flag.is_cancelled());
        drop(owner);
        assert!(flag.is_cancelled());
    }
    #[test]
    #[ignore = "explicit loopback HTTP player acceptance; ordinary tests need no network"]
    fn native_player_http_input_retains_file_for_reopen_and_seek() {
        use std::io::{Read, Write};
        let fixture = include_bytes!("../../../tests/fixtures/playback-errors/ffv1-gray-8.mkv");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                fixture.len()
            )
            .unwrap();
            socket.write_all(fixture).unwrap();
        });
        let mut player = Player {
            no_audio: true,
            ..Default::default()
        };
        player
            .open(format!("http://{address}/video.mkv").into())
            .unwrap();
        server.join().unwrap();
        assert_eq!(player.name, "video.mkv");
        assert_eq!(player.dimensions, [4, 3]);
        assert!(player.seekable);
        let path = player.opened.clone().unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), fixture);
        // Reopen the downloaded path without contacting the server or losing ownership.
        player.open(path.clone()).unwrap();
        assert!(player.network_input.is_some());
        let mut reader = NativeReader::without_memory_limit(std::io::BufReader::new(
            std::fs::File::open(&path).unwrap(),
        ))
        .unwrap();
        assert!(reader.read_frame_raw().unwrap().is_some());
        assert!(
            reader
                .seek_raw(Duration::from_millis(40))
                .unwrap()
                .is_some()
        );
        let (start, _, scale) = reader.frame_interval().unwrap();
        assert_eq!(start * 1000 / u128::from(scale), 40);
        reader.rewind().unwrap();
        assert!(reader.read_frame_raw().unwrap().is_some());
        drop(reader);
        drop(player);
        assert!(!path.exists());
    }
    #[test]
    #[ignore = "explicit loopback HLS playback; ordinary tests need no network"]
    fn native_player_hls_decodes_all_frames_and_seeks() {
        use std::io::{Read, Write};
        let manifest = include_bytes!("../../../tests/fixtures/playback-errors/hls/byterange.m3u8");
        let object = include_bytes!("../../../tests/fixtures/playback-errors/hls/objects.mp4");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            for body in [manifest.as_slice(), object.as_slice()] {
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(body).unwrap();
            }
        });
        let mut player = Player {
            no_audio: true,
            ..Default::default()
        };
        player
            .open(format!("http://{address}/video.m3u8").into())
            .unwrap();
        server.join().unwrap();
        assert_eq!(player.dimensions, [320, 240]);
        assert!(player.seekable);
        let path = player.opened.clone().unwrap();
        let mut reader = NativeReader::without_memory_limit(std::io::BufReader::new(
            std::fs::File::open(&path).unwrap(),
        ))
        .unwrap();
        let mut count = 0;
        while reader.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 25);
        assert!(reader
            .seek_raw(Duration::from_millis(400))
            .unwrap()
            .is_some());
        reader.rewind().unwrap();
        assert!(reader.read_frame_raw().unwrap().is_some());
        drop(reader);
        drop(player);
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod ogg_opus_track_dispatch_tests {
    #[test]
    fn ogg_opus_track_selection_reaches_the_player_factory() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/opus-surround-parallel.opus");
        for (track, skip, samples) in [(0, 312, 5311), (1, 648, 4975)] {
            let mut stream = super::ogg_audio_stream(&path, track).unwrap();
            assert_eq!(stream.codec(), "A_OPUS");
            assert_eq!(stream.audio_tracks().len(), 2);
            let head = fvid_opus::OpusHead::parse(stream.extra_data()).unwrap();
            assert_eq!(head.pre_skip, skip);
            assert_eq!(
                stream.duration().unwrap().as_nanos(),
                samples as u128 * 1_000_000_000 / 48000
            );
            assert_eq!(stream.next_packet().unwrap().unwrap().pts, -i64::from(skip));
        }
        assert!(super::ogg_audio_stream(&path, 2).is_none());
    }
}

#[cfg(test)]
mod default_track_selection_regressions {
    use super::*;
    #[test]
    fn automatic_audio_open_uses_container_preference_in_the_player() {
        for file in ["matroska-default-tracks/audiovisual.mkv","mp4-enabled-tracks/audiovisual.mp4"] {
            let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors").join(file);
            let mut player=Player::default();
            assert!(player.open_item_audio(&path));
            assert_eq!((player.audio_tracks.len(),player.audio_track),(2,1),"{file}");
        }
    }
}

#[cfg(test)]
mod preferred_audio_worker_regressions {
    use super::*;
    use std::sync::{Arc,Mutex};
    struct Capture {
        packets:Arc<Mutex<Vec<crate::audio::AudioPacket>>>,
        spec:Option<crate::audio::AudioSpec>,
        at:Duration,
    }
    impl crate::audio::AudioBackend for Capture {
        fn start(&mut self,spec:crate::audio::AudioSpec)->Result<(),crate::audio::AudioError>{self.spec=Some(spec);Ok(())}
        fn push(&mut self,packet:crate::audio::AudioPacket)->Result<(),crate::audio::AudioError>{let spec=self.spec.unwrap();self.at=packet.presentation_time()+Duration::from_secs_f64((packet.data.len()/4/usize::from(spec.channels)) as f64/f64::from(spec.sample_rate));self.packets.lock().unwrap().push(packet);Ok(())}
        fn position(&self)->Duration{self.at}
        fn flush(&mut self,at:Duration)->Result<(),crate::audio::AudioError>{self.at=at;self.packets.lock().unwrap().clear();Ok(())}
        fn pause(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn resume(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn stop(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
    }
    fn ended(playback:&crate::audio_thread::AudioPlayback,generation:u64){
        let until=Instant::now()+Duration::from_secs(5);
        loop {assert!(Instant::now()<until,"preferred audio did not reach EOF");match playback.poll(){Some(crate::audio_thread::AudioEvent::Ended(g)) if g==generation=>return,Some(crate::audio_thread::AudioEvent::Error(e))=>panic!("{e}"),_=>{}}std::thread::sleep(Duration::from_millis(1));}
    }
    #[test]
    fn actual_preferred_audio_worker_preserves_pcm_clock_rewind_and_seek() {
        let fixtures = [
            ("matroska-default-tracks", "audiovisual.mkv", "expected-audio.f32le", 1, 2, 60, 40),
            ("mp4-enabled-tracks", "audiovisual.mp4", "expected-audio.f32le", 1, 2, 60, 40),
            ("matroska-pcm-clock", "clock.mkv", "clock.f32le", 0, 1, 60, 40),
            ("matroska-pcm-clock", "big.mkv", "big.f32le", 0, 1, 60, 40),
            ("matroska-pcm-clock", "float.mkv", "float.f32le", 0, 1, 60, 40),
            ("matroska-pcm-clock", "negative.mkv", "negative.f32le", 0, 1, 50, 30),
            ("mp4-audio-clocks", "pcm-8000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "pcm-1000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "pcm-44100.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "pcm-48000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "pcm-90000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "pcm-1000-negative.mp4", "negative-expected.f32le", 0, 1, 50, 30),
            ("mp4-audio-clocks", "alac-8000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "alac-1000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "alac-44100.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "alac-48000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "alac-90000.mp4", "expected.f32le", 0, 1, 60, 40),
            ("mp4-audio-clocks", "alac-1000-negative.mp4", "negative-expected.f32le", 0, 1, 50, 30),
        ];
        for (folder, file, oracle, chosen, count, end_ms, seek_ms) in fixtures {
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors").join(folder);
            let path = base.join(file);
            let expected = std::fs::read(base.join(oracle)).unwrap();
            let stream = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, None)).unwrap();
            assert_eq!(stream.audio_track_index(), chosen);
            assert_eq!(stream.audio_tracks().len(), count);
            let packets = Arc::new(Mutex::new(Vec::new()));
            let captured = packets.clone();
            let mut playback = crate::audio_thread::AudioPlayback::start(stream, move ||
                Box::new(Capture { packets: captured, spec: None, at: Duration::ZERO }));
            for generation in 0..2 {
                if generation != 0 { playback.rewind(); }
                playback.play();
                ended(&playback, generation);
                let actual: Vec<_> = packets.lock().unwrap().iter().flat_map(|p| p.data.iter().copied()).collect();
                assert_eq!(actual, expected, "{file}, generation{generation}");
                assert_eq!(playback.position(), Duration::from_millis(end_ms), "{file}");
            }
            playback.seek(Duration::from_millis(seek_ms));
            playback.play();
            ended(&playback, 2);
            let actual: Vec<_> = packets.lock().unwrap().iter().flat_map(|p| p.data.iter().copied()).collect();
            assert_eq!(actual, expected[expected.len()-160*4..], "{file} seek");
            assert_eq!(playback.position(), Duration::from_millis(end_ms), "{file} seek");
        }
    }
    #[test]
    fn explicit_audio_choice_and_invalid_index_fallback_remain_compatible(){
        let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-default-tracks/audiovisual.mkv");
        for (requested,chosen) in [(None,1),(Some(0),0),(Some(1),1),(Some(9),0)]{let mut player=Player{preferred_audio:requested,..Default::default()};assert!(player.open_item_audio(&path));assert_eq!(player.audio_track,chosen);assert_eq!(player.audio_tracks.len(),2);}
        let mut player=Player{no_audio:true,..Default::default()};assert!(!player.open_item_audio(&path));assert!(player.audio.is_none());
    }
}

#[cfg(test)]
#[test]
fn parallel_vorbis_player_factory_selects_the_second_track() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/vorbis-timestamps-parallel.ogg");
    let mut stream = ogg_audio_stream(&path, 1).expect("second Vorbis track");
    assert_eq!(stream.codec(), "A_VORBIS");
    assert_eq!(stream.audio_tracks().len(), 2);
    assert_eq!(stream.audio_track_index(), 1);
    let mut decoder = stream.make_decoder().unwrap();
    let mut frames = 0;
    while let Some(step) = stream.next_step().unwrap() { match step {
        crate::audio::AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
        crate::audio::AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
        crate::audio::AudioStep::Encoded(packet) => {
            if let Some(decoded) = decoder.decode_packet(&packet.data,packet.pts,packet.duration as u64).unwrap() {
                if let Some(pcm) = stream.present_decoded(decoded.packet,decoded.source_pts).unwrap() { frames += pcm.data.len()/8; }
            }
        },
        _ => panic!("unexpected Vorbis step"),
    }}
    assert_eq!(frames,394);
}

#[cfg(test)]
#[test]
fn mixed_ogg_player_catalogue_routes_both_codecs_in_bos_order(){
    for(name,first)in [("ogg-mixed-opus-first.ogg","A_OPUS"),("ogg-mixed-vorbis-first.ogg","A_VORBIS")]{
        let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors").join(name);
        for index in 0..2{
            let mut stream=ogg_audio_stream(&path,index).expect("mixed Ogg track");assert_eq!(stream.audio_tracks().len(),2);assert_eq!(stream.audio_track_index(),index);
            let expected=if index==0{first}else if first=="A_OPUS"{"A_VORBIS"}else{"A_OPUS"};assert_eq!(stream.codec(),expected);
            let stride=usize::from(stream.channels())*4;let mut decoder=stream.make_decoder().unwrap();let mut frames=0;
            while let Some(step)=stream.next_step().unwrap(){match step{
                crate::audio::AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
                crate::audio::AudioStep::ResetDecoder=>stream.reset_decoder(&mut decoder).unwrap(),
                crate::audio::AudioStep::Encoded(packet)=>{if let Some(decoded)=decoder.decode_packet(&packet.data,packet.pts,packet.duration as u64).unwrap(){if let Some(pcm)=stream.present_decoded(decoded.packet,decoded.source_pts).unwrap(){frames+=pcm.data.len()/stride;}}},
                _=>panic!("unexpected mixed audio step"),
            }}assert_eq!(frames,if expected=="A_OPUS"{5311}else{411});
        }
    }
}

#[cfg(test)]
#[test]
fn player_automatic_audio_choice_skips_unsupported_default_pcm_width() {
    for (folder,name,extension) in [
        ("matroska-pcm-choice","integer-default","mkv"),("matroska-pcm-choice","float-default","mkv"),
        ("matroska-pcm-choice","opus-missing-setup","mkv"),("matroska-pcm-choice","aac-missing-setup","mkv"),
        ("matroska-pcm-choice","vorbis-missing-setup","mkv"),
        ("mp4-pcm-choice","unsupported-movie","mp4"),("mp4-pcm-choice","both-movie","mp4"),
    ] {
        let path=Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors").join(folder).join(format!("{name}.{extension}"));
        let mut player=Player::default();
        assert!(player.open_item_audio(&path));
        assert_eq!(player.audio_track,1,"{name}");assert_eq!(player.audio_tracks.len(),2);
    }
}

#[cfg(test)]
mod audio_duration_publication_regressions {
    use super::*;
    #[test]
    fn completed_audio_duration_reaches_the_sound_only_player_timeline() {
        let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/audio-duration-publication/tail.mkv");
        let source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(std::fs::read(path).unwrap()),Default::default()).unwrap();
        let audio=crate::audio_thread::AudioPlayback::start(Box::new(source),||Box::new(crate::audio::NullBackend::default()));
        assert_eq!(audio.duration(),Some(Duration::from_millis(60)));
        let mut player=Player::default();player.duration=audio.duration();player.audio=Some(audio);player.ended=true;
        player.audio.as_ref().unwrap().play();let deadline=Instant::now()+Duration::from_secs(5);
        while !player.audio_ended {assert!(Instant::now()<deadline,"audio did not finish");player.poll_audio();std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_millis(20)));
        assert_eq!(player.duration,Some(Duration::from_millis(20)),"sound timeline retained initial snapshot");
    }
}

#[cfg(test)]
mod audio_restart_preroll_regressions {
    use super::*;
    use std::sync::{Arc,Mutex,atomic::{AtomicBool,AtomicUsize,Ordering}};
    struct Device {pcm:Arc<Mutex<Vec<u8>>>,resumes:Arc<AtomicUsize>,flushed:Arc<AtomicBool>,at:Duration}
    impl crate::audio::AudioBackend for Device {
        fn start(&mut self,_:crate::audio::AudioSpec)->Result<(),crate::audio::AudioError>{Ok(())}
        fn push(&mut self,p:crate::audio::AudioPacket)->Result<(),crate::audio::AudioError>{self.at=p.presentation_time()+Duration::from_nanos(p.data.len() as u64/4*1_000_000_000/8000);self.pcm.lock().unwrap().extend(p.data);Ok(())}
        fn position(&self)->Duration{self.at}
        fn flush(&mut self,at:Duration)->Result<(),crate::audio::AudioError>{self.at=at;self.pcm.lock().unwrap().clear();self.flushed.store(true,Ordering::Release);Ok(())}
        fn pause(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn resume(&mut self)->Result<(),crate::audio::AudioError>{self.resumes.fetch_add(1,Ordering::AcqRel);Ok(())}
        fn stop(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
    }
    #[test]
    fn paused_audio_rewind_waits_for_an_explicit_play_command() {
        let bytes=include_bytes!("../../../tests/fixtures/playback-errors/audio-restart-preroll/restart.mkv");let expected=include_bytes!("../../../tests/fixtures/playback-errors/audio-restart-preroll/expected.f32le");
        let source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(bytes.as_slice()),Default::default()).unwrap();
        let pcm=Arc::new(Mutex::new(Vec::new()));let resumes=Arc::new(AtomicUsize::new(0));let flushed=Arc::new(AtomicBool::new(false));let device=Device{pcm:pcm.clone(),resumes:resumes.clone(),flushed:flushed.clone(),at:Duration::ZERO};
        let mut audio=crate::audio_thread::AudioPlayback::start(Box::new(source),move||Box::new(device));audio.rewind();
        let until=Instant::now()+Duration::from_secs(5);while !flushed.load(Ordering::Acquire){assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
        std::thread::sleep(Duration::from_millis(100));assert!(pcm.lock().unwrap().is_empty());assert_eq!(resumes.load(Ordering::Acquire),0);
        audio.play();let until=Instant::now()+Duration::from_secs(5);
        loop {assert!(Instant::now()<until);match audio.poll(){Some(crate::audio_thread::AudioEvent::Ended(g)) if g==1=>break,Some(crate::audio_thread::AudioEvent::Error(e))=>panic!("{e}"),_=>{}}std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(pcm.lock().unwrap().as_slice(),expected);assert_eq!(resumes.load(Ordering::Acquire),1);
    }
    #[test]
    fn restarted_video_holds_audio_until_the_first_picture_is_ready() {
        for (seek,paused,resume_early) in [(false,false,false),(true,false,false),(true,true,false),(true,true,true)] {
        let bytes=include_bytes!("../../../tests/fixtures/playback-errors/audio-restart-preroll/restart.mkv");let expected=include_bytes!("../../../tests/fixtures/playback-errors/audio-restart-preroll/expected.f32le");
        let mut reader=NativeReader::software(std::io::Cursor::new(bytes.as_slice()),32<<20).unwrap();
        for _ in 0..3 {assert!(reader.read_frame_raw().unwrap().is_some());}assert!(reader.read_frame_raw().unwrap().is_none());
        let mut reader=NativeReader::software(std::io::Cursor::new(bytes.as_slice()),32<<20).unwrap();assert!(reader.read_frame().unwrap());let playback=Playback::start(reader,None);
        let source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(bytes.as_slice()),Default::default()).unwrap();
        let pcm=Arc::new(Mutex::new(Vec::new()));let resumes=Arc::new(AtomicUsize::new(0));let flushed=Arc::new(AtomicBool::new(false));let device=Device{pcm:pcm.clone(),resumes:resumes.clone(),flushed:flushed.clone(),at:Duration::ZERO};
        let audio=crate::audio_thread::AudioPlayback::start(Box::new(source),move||Box::new(device));audio.play();
        let until=Instant::now()+Duration::from_secs(5);
        loop {assert!(Instant::now()<until);match audio.poll(){Some(crate::audio_thread::AudioEvent::Ended(_))=>break,Some(crate::audio_thread::AudioEvent::Error(e))=>panic!("{e}"),_=>{}}std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(pcm.lock().unwrap().as_slice(),expected);assert_eq!(resumes.load(Ordering::Acquire),1);
        let mut player=Player{audio:Some(audio),playback:Some(playback),ended:true,audio_ended:true,paused,period:Duration::from_millis(20),..Default::default()};if seek {player.seek_time(Duration::from_millis(20));}else{player.restart();}
        let until=Instant::now()+Duration::from_secs(5);while !flushed.load(Ordering::Acquire){assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
        std::thread::sleep(Duration::from_millis(100));
        assert!(pcm.lock().unwrap().is_empty(),"navigation decoded audio before video preroll completed: seek={seek}");assert_eq!(resumes.load(Ordering::Acquire),1);assert_eq!(player.audio.as_ref().unwrap().position(),if seek{Duration::from_millis(20)}else{Duration::ZERO});
        if resume_early {player.toggle_pause();std::thread::sleep(Duration::from_millis(100));assert!(pcm.lock().unwrap().is_empty(),"resume during video preroll released audio early");assert_eq!(resumes.load(Ordering::Acquire),1);}
        let ctx=egui::Context::default();let until=Instant::now()+Duration::from_secs(5);
        while player.buffering {assert!(Instant::now()<until);player.present(&ctx);std::thread::sleep(Duration::from_millis(1));}
        if paused && !resume_early {assert!(player.paused);std::thread::sleep(Duration::from_millis(20));assert!(pcm.lock().unwrap().is_empty());assert_eq!(resumes.load(Ordering::Acquire),1);player.toggle_pause();}
        let until=Instant::now()+Duration::from_secs(5);let wanted=if seek{&expected[640..]}else{expected.as_slice()};while pcm.lock().unwrap().len()!=wanted.len(){assert!(Instant::now()<until);player.poll_audio();std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(pcm.lock().unwrap().as_slice(),wanted);assert_eq!(resumes.load(Ordering::Acquire),2);
        }
    }
}

#[cfg(test)]
mod wav_unsigned8_factory_regressions {
    use super::*;
    #[test]
    fn player_audio_factory_admits_unsigned8_wave_track_geometry() {
        for (name,rate,channels) in [("mono",8000,1),("stereo",44100,2),("three-channel",48000,3),("extensible",8000,2)] {
            let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wav-unsigned8").join(format!("{name}.wav"));
            let mut player=Player::default();assert!(player.open_item_audio(&path),"{name}");
            assert_eq!(player.audio_tracks.len(),1);assert_eq!((player.audio_tracks[0].sample_rate,player.audio_tracks[0].channels),(rate,channels));
            assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_millis(20)));
        }
    }
}

#[cfg(test)]
mod wav_valid_bits_factory_regressions {
    use super::*;
    #[test]
    fn player_factory_admits_left_aligned_extensible_pcm_storage_widths() {
        for (name,channels) in [("pcm12-in-16",1),("pcm20-in-24",2),("pcm24-in-32",1),("pcm16-in-32",2),("pcm8-in-16",1),("pcm7-in-8",2)] {
            let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wav-valid-bits").join(format!("{name}.wav"));
            let mut player=Player::default();assert!(player.open_item_audio(&path),"{name}");
            assert_eq!(player.audio_tracks[0].channels,channels);assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_millis(20)));
        }
    }
}

#[cfg(test)]
mod audio_chunk_topology_factory_regressions {
    use super::*;
    #[test]
    fn player_factory_refuses_partial_ambiguous_chunk_payloads_and_keeps_normal_controls() {
        let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/audio-duplicate-chunks");
        for file in ["wave-split-data.wav","wave-data-plus-wavl.wav","aiff-split-sound.aiff"] {
            let mut player=Player::default();assert!(!player.open_item_audio(&base.join(file)),"{file} partial source admitted");assert!(player.audio.is_none());
        }
        for file in ["wave-control.wav","wave-info-list.wav","aiff-control.aiff"] {
            let mut player=Player::default();assert!(player.open_item_audio(&base.join(file)),"{file} normal source refused");assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_millis(20)));
        }
    }
}

#[cfg(test)]
mod pcm_run_alignment_factory_regressions {
    use super::*;
    #[test]
    fn player_factory_does_not_publish_aligned_prefixes_of_malformed_fixed_frame_runs() {
        let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/pcm-run-alignment");
        for file in ["wav-int16-orphan-1.wav","au-float64-orphan-7.au"] {
            let mut player=Player::default();assert!(!player.open_item_audio(&base.join(file)),"{file} malformed source admitted");assert!(player.audio.is_none());
        }
        for file in ["wav-odd-mono8.wav","au-odd-mono8.au"] {
            let mut player=Player::default();assert!(player.open_item_audio(&base.join(file)),"{file} padding/annotation control refused");assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_nanos(20_125_000)));
        }
    }
}

#[cfg(test)]
mod audio_truncated_eof_player_regressions {
    use super::*;
    #[test]
    fn known_incomplete_audio_removes_the_failed_handle_while_healthy_eof_remains_replayable() {
        let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/audio-truncated-eof");
        for (file,truncated) in [("wav-truncated.wav",true),("au-truncated.au",true),("wav-unknown.wav",false),("au-unknown-zero.au",false),("aiff-truncated.aiff",true),("aiff-prefix.aiff",false),("aiff-zero-claim.aiff",false)] {
            let bytes=std::fs::read(base.join(file)).unwrap();let source:Box<dyn crate::audio::AudioStream>=if file.starts_with("wav"){Box::new(crate::playback_wav::WavAudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap())}else if file.starts_with("aiff"){Box::new(crate::playback_aiff::AiffAudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap())}else{Box::new(crate::playback_au::AuAudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap())};
            let audio=crate::audio_thread::AudioPlayback::start(source,||Box::new(crate::audio::NullBackend::default()));
            let mut player=Player{audio:Some(audio),ended:true,audio_ended:false,..Default::default()};player.audio.as_ref().unwrap().play();let until=Instant::now()+Duration::from_secs(5);
            while !player.audio_ended {assert!(Instant::now()<until,"{file} no audio completion");player.poll_audio();std::thread::sleep(Duration::from_millis(1));}
            assert_eq!(player.audio.is_none(),truncated,"{file} failed versus healthy EOF route");
        }
    }
}

#[cfg(test)]
mod timeline_release_duration_factory_regressions {
    use super::*;
    #[test]
    fn player_audio_duration_covers_the_complete_synthesized_release() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for (file,millis) in [("midi/two-notes.mid",2200),("xm/two-notes.xm",480)] {
            let mut player=Player::default();assert!(player.open_item_audio(&root.join(file)),"{file}");
            assert_eq!(player.audio.as_ref().unwrap().duration(),Some(Duration::from_millis(millis)));
        }
    }
}

mod subtitle_paint_tests;

#[cfg(test)]
mod native_wavpack_file_factory_regressions {
    use super::*;
    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wavpack-files")
    }
    fn pcm(mut stream: Box<dyn crate::audio::AudioStream>) -> Vec<u8> {
        assert_eq!(stream.codec(), "wavpack");
        let mut decoder = stream.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = stream.next_packet().unwrap() {
            let decoded = decoder.decode_packet(&packet.data, packet.pts, packet.duration as u64).unwrap().unwrap();
            if let Some(packet) = stream.present_decoded(decoded.packet, decoded.source_pts).unwrap() {
                out.extend(packet.data);
            }
        }
        stream.validate_eof().unwrap();
        out
    }
    #[test]
    fn automatic_native_file_dispatch_preserves_exact_pcm() {
        for name in ["false-d8-t1-neg0-none", "correction-8-mono-snr-none-cross0-term1", "dsd-high-six-p3-noise", "float-hybrid-stereo-bitrate-delta-optimized-e127"] {
            let path = root().join(format!("{name}.wv"));
            let stream = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, None)).expect(name);
            assert_eq!(pcm(stream), std::fs::read(root().join(format!("{name}.f32le"))).unwrap(), "{name}");
        }
    }
    #[test]
    fn correction_selection_and_invalid_companion_are_strict() {
        let name = "correction-8-mono-snr-none-cross0-term1";
        let expected = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        assert_eq!(pcm(wavpack_audio_stream(&root().join(format!("{name}.wvc")), 0).unwrap()), expected);
        assert!(wavpack_audio_stream(&root().join(format!("{name}.wv")), 1).is_none());
        let temp = std::env::temp_dir().join(format!("fvid-native-wavpack-factory-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let result = std::panic::catch_unwind(|| {
            let main = temp.join("owned.WV");
            let correction = temp.join("owned.WVC");
            std::fs::copy(root().join(format!("{name}.wv")), &main).unwrap();
            std::fs::copy(root().join(format!("{name}.wvc")), &correction).unwrap();
            assert_eq!(pcm(wavpack_audio_stream(&correction, 0).unwrap()), expected);
            std::fs::write(&correction, b"invalid correction").unwrap();
            assert!(wavpack_audio_stream(&main, 0).is_none());
            assert!(AUDIO_READERS.iter().find_map(|attempt| attempt.open(&main, None)).is_none());
        });
        std::fs::remove_dir_all(&temp).unwrap();
        result.unwrap();
    }
}

#[cfg(test)]
mod player_queue_and_geometry_regressions {
    use super::*;

    #[test]
    fn folder_expansion_and_playlists_admit_avi_and_wavpack_and_ignore_ini_headers_and_quotes() {
        let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/player-queue");
        let temp = std::env::temp_dir().join(format!("fvid-player-queue-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        for name in ["queue.pls", "queue.m3u", "clip.avi", "clip.wv", "companion.wvc"] {
            std::fs::copy(fixture_dir.join(name), temp.join(name)).unwrap();
        }
        // Also place a stray file literally named "[playlist]" in the directory
        // to prove PLS section headers are never resolved as relative paths.
        std::fs::write(temp.join("[playlist]"), b"not media").unwrap();

        let expanded_dir = expand_inputs(&[temp.clone()]);
        assert_eq!(
            expanded_dir,
            vec![
                temp.join("clip.avi"),
                temp.join("clip.wv"),
                temp.join("companion.wvc"),
            ]
        );

        let expanded_pls = expand_inputs(&[temp.join("queue.pls")]);
        assert_eq!(
            expanded_pls,
            vec![temp.join("clip.avi"), temp.join("clip.wv")]
        );

        let expanded_m3u = expand_inputs(&[temp.join("queue.m3u")]);
        assert_eq!(
            expanded_m3u,
            vec![temp.join("clip.avi"), temp.join("clip.wv")]
        );

        std::fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn subtitle_negative_delay_and_zero_sized_video_rect_do_not_overflow_or_produce_nan() {
        let mut player = Player {
            subtitle_shown: true,
            subtitle_delay_ms: -5_000,
            cues: vec![Cue {
                start: Duration::from_secs(1),
                end: Duration::from_secs(2),
                text: "caption".into(),
            }],
            ..Default::default()
        };
        assert_eq!(player.subtitle_line(Duration::MAX), None);
        player.subtitle_delay_ms = 5_000;
        assert_eq!(player.subtitle_line(Duration::from_secs(1)), None);

        let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 360.0));
        let rect = video_rect(frame, Vec2::ZERO, 1_000, Vec2::new(10.0, -10.0));
        assert!(rect.min.x.is_finite() && rect.min.y.is_finite());
        assert!(rect.max.x.is_finite() && rect.max.y.is_finite());
        assert_eq!(rect.size(), Vec2::ZERO);
    }

    #[test]
    fn oversized_lut_and_skin_files_are_refused_by_bounded_readers() {
        let temp = std::env::temp_dir().join(format!("fvid-player-bounds-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        let huge_skin = temp.join("huge-skin.json");
        std::fs::write(&huge_skin, vec![b' '; 64 * 1024 + 2]).unwrap();
        let skin_err = player_skin::Skin::load(huge_skin.to_str().unwrap()).unwrap_err();
        assert!(
            skin_err.to_string().contains("too large"),
            "unexpected skin error: {skin_err}"
        );

        let huge_lut = temp.join("huge.cube");
        std::fs::write(&huge_lut, vec![b'#'; 16 * 1024 * 1024 + 2]).unwrap();
        let lut_err = read_lut(huge_lut.to_str().unwrap()).unwrap_err();
        assert!(
            lut_err.to_string().contains("too large"),
            "unexpected lut error: {lut_err}"
        );

        std::fs::remove_dir_all(&temp).unwrap();
    }
}


#[cfg(test)]
mod elementary_programme_factory_regressions {
    use super::*;
    use crate::audio::{AudioStep,AudioStream};
    fn pcm(stream:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{
        let mut decoder=stream.make_decoder().unwrap();let mut out=Vec::new();
        while let Some(step)=stream.next_step().unwrap(){
            if let AudioStep::Encoded(p)=step {
                if let Some(p)=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap(){
                    if let Some(p)=stream.present_decoded(p.packet,p.source_pts).unwrap(){out.push((p.presentation_time(),p.data));}
                }
            }
        }
        while let Some(p)=decoder.finish_packet().unwrap(){if let Some(p)=stream.present_decoded(p.packet,p.source_pts).unwrap(){out.push((p.presentation_time(),p.data));}}
        stream.validate_eof().unwrap();out
    }
    #[test]
    fn player_factory_exposes_independent_programmes_with_their_own_geometry_and_pcm(){
        let directory=std::env::temp_dir().join(format!("fvid-eac3-programme-factory-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&directory).unwrap();
        let source=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-programme-reader");
        let path=directory.join("multiplexed.ec3");std::fs::copy(source.join("heterogeneous.ec3"),&path).unwrap();
        let preferred=AUDIO_READERS.iter().find_map(|attempt|attempt.open(&path,None)).unwrap();assert_eq!(preferred.audio_track_index(),0);assert_eq!(preferred.audio_tracks().len(),8);drop(preferred);
        for nth in 0..8 {
            let mut expected=crate::playback_ac3::Ac3AudioReader::open(std::io::Cursor::new(std::fs::read(source.join(format!("heterogeneous-p{nth}-control.ec3"))).unwrap()),Default::default()).unwrap();
            let words=vec![format!("--audio-track={}",nth+1),path.to_string_lossy().into_owned()];
            let parsed=super::parse_play_args(&words).unwrap();assert_eq!(parsed.audio_track,Some(nth));
            let mut chosen=AUDIO_READERS.iter().find_map(|attempt|attempt.open(&path,parsed.audio_track)).unwrap();
            assert_eq!(chosen.audio_track_index(),nth);assert_eq!(chosen.audio_tracks().len(),8);
            assert_eq!((chosen.sample_rate(),chosen.channels(),chosen.speaker_mask().unwrap()),(expected.sample_rate(),expected.channels(),expected.speaker_mask().unwrap()));
            assert!(pcm(&mut *chosen)==pcm(&mut expected),"programme{nth}: player factory PCM differs");
        }
        assert!(AUDIO_READERS.iter().find_map(|attempt|attempt.open(&path,Some(8))).is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod mp4_frame_step_regressions;

mod stop_bound_regressions;

mod stop_clock_regressions;

mod seek_failure_regressions;

mod audio_handoff_regressions;

mod rate_clock_regressions;

mod jump_clock_regressions;

mod seek_phase_regressions;

mod late_clock_regressions;

mod ab_clock_regressions;

mod chapter_clock_regressions;

mod audio_failure_clock_regressions;

mod io_clock_regressions;

mod sound_stop_loop_regressions;

mod sound_eof_loop_regressions;

mod audio_seek_intent_regressions;

mod track_clock_regressions;

mod track_preroll_regressions;

mod track_source_lifetime_regressions;

mod subtick_video_edit_regressions;

mod short_track_clock_regressions;

#[cfg(test)]
mod native_wave64_file_regressions {
    use super::*;
    #[test]
    fn wave64_paths_and_native_factory_preserve_pcm() {
        assert!(playable_path(Path::new("synthetic.w64")));
        assert!(playable_path(Path::new("synthetic.W64")));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wave64-playback");
        let path = root.join("classic-t1-b16-c2-base.w64");
        assert_eq!(expand_inputs(&[path.clone()]), vec![path.clone()]);
        let mut stream = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, None)).expect("native Wave64 factory");
        let mut decoder = stream.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = stream.next_packet().unwrap() {
            if let Some(decoded) = decoder.decode_packet(&packet.data, stream.decoder_stamp(&packet), packet.duration as u64).unwrap() {
                if let Some(packet) = stream.present_decoded(decoded.packet, decoded.source_pts).unwrap() { out.extend(packet.data); }
            }
        }
        stream.validate_eof().unwrap();
        assert_eq!(out, std::fs::read(root.join("classic-t1-b16-c2.f32le")).unwrap());
    }
}

#[cfg(test)]
mod native_bw64_file_regressions {
    use super::*;
    #[test]
    fn bw64_native_factory_preserves_pcm() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/bw64-playback");
        let path = root.join("classic-t1-b16-c2-base.bw64.wav");
        assert_eq!(expand_inputs(&[path.clone()]), vec![path.clone()]);
        let mut stream = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, None)).expect("native BW64 factory");
        let mut decoder = stream.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = stream.next_packet().unwrap() {
            if let Some(decoded) = decoder.decode_packet(&packet.data, stream.decoder_stamp(&packet), packet.duration as u64).unwrap() {
                if let Some(packet) = stream.present_decoded(decoded.packet, decoded.source_pts).unwrap() { out.extend(packet.data); }
            }
        }
        stream.validate_eof().unwrap();
        assert_eq!(out, std::fs::read(root.join("classic-t1-b16-c2.f32le")).unwrap());
    }
}

#[cfg(test)]
mod native_wave_ima_file_regressions {
    use super::*;
    #[test]
    fn automatic_wave_ima_factory_preserves_trimmed_pcm() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wave-ima-playback");
        for kind in ["riff", "rf64", "wave64"] {
            let path = root.join(format!("c2-b16-trim-{kind}.wav"));
            let mut stream = AUDIO_READERS.iter().find_map(|a|a.open(&path,None)).expect("native IMA WAV");
            assert_eq!(stream.codec(), "adpcm_ima_wav");
            let mut decoder = stream.make_decoder().unwrap();
            let mut out = Vec::new();
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(d) = decoder.decode_packet(&p.data,stream.decoder_stamp(&p),p.duration as u64).unwrap() {
                    if let Some(p) = stream.present_decoded(d.packet,d.source_pts).unwrap() {out.extend(p.data);}
                }
            }
            stream.validate_eof().unwrap();
            assert_eq!(out,std::fs::read(root.join("c2-b16-trim.f32le")).unwrap(),"{kind}");
        }
    }
}

#[cfg(test)]
mod native_wave_ms_adpcm_file_regressions {
    use super::*;
    #[test]
    fn automatic_wave_ms_factory_preserves_extended_coefficients_and_tail() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wave-ms-adpcm-playback");
        for table in ["standard", "extended"] {
            for kind in ["riff", "rf64", "wave64"] {
                let path = root.join(format!("c2-b16-{table}-trim-{kind}.wav"));
                let mut stream = AUDIO_READERS.iter().find_map(|a|a.open(&path,None)).expect("native MS ADPCM WAVE");
                assert_eq!(stream.codec(), "adpcm_ms");
                let mut decoder = stream.make_decoder().unwrap();
                let mut out = Vec::new();
                while let Some(p) = stream.next_packet().unwrap() {
                    if let Some(d) = decoder.decode_packet(&p.data,stream.decoder_stamp(&p),p.duration as u64).unwrap() {
                        if let Some(p) = stream.present_decoded(d.packet,d.source_pts).unwrap() {out.extend(p.data);}
                    }
                }
                stream.validate_eof().unwrap();
                assert_eq!(out,std::fs::read(root.join(format!("c2-b16-{table}-trim.f32le"))).unwrap(),"{table}/{kind}");
            }
        }
    }
}

#[cfg(test)]
mod native_avi_adpcm_start_file_regressions {
    use super::*;
    #[test]
    fn automatic_avi_factory_keeps_delayed_native_clocks_and_pcm() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avi-adpcm-start");
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        for case in manifest["cases"].as_array().unwrap().iter().filter(|c| c["start"] == 73 && c["clock_mode"] == "equivalent") {
            let path = root.join(case["file"].as_str().unwrap());
            let origin = case["origin"].as_u64().unwrap();
            let mut stream = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, None)).expect("native delayed AVI");
            assert_eq!(stream.duration(), Some(crate::audio::sample_duration(origin + case["frames"].as_u64().unwrap(), 8000)));
            let mut decoder = stream.make_decoder().unwrap();
            let mut out = Vec::new();
            let mut first = true;
            while let Some(packet) = stream.next_packet().unwrap() {
                if first { assert_eq!(packet.pts, origin as i64); first = false; }
                if let Some(decoded) = decoder.decode_packet(&packet.data, stream.decoder_stamp(&packet), packet.duration as u64).unwrap() {
                    if let Some(pcm) = stream.present_decoded(decoded.packet, decoded.source_pts).unwrap() { out.extend(pcm.data); }
                }
            }
            assert!(!first);
            stream.validate_eof().unwrap();
            assert_eq!(out, std::fs::read(root.join(case["expected"].as_str().unwrap())).unwrap());
        }
    }
}

mod hevc_start_seek_tests;
