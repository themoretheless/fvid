use std::path::{Path, PathBuf};

#[test]
fn rate_scales_time_and_audio_phase() {
    let media = fvid_media::clamp_rate_milli;
    assert_eq!(media(1.0), 1_000);
    assert_eq!(media(0.1), 250);
    assert_eq!(media(8.0), 4_000);
    assert_eq!(media(f32::NAN), 1_000);
    assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 1_000), 1_000_000);
    assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 2_000), 2_000_000);
    assert_eq!(fvid_media::scale_elapsed_us(1_000_000, 250), 250_000);
    assert_eq!(fvid_media::advance_rate_phase(0, 1_000), (1, 0));
    assert_eq!(fvid_media::advance_rate_phase(0, 2_000), (2, 0));
    let mut phase = 0;
    let mut consumed = 0u32;
    for _ in 0..4 {
        let (need, next) = fvid_media::advance_rate_phase(phase, 250);
        consumed += need;
        phase = next;
    }
    assert_eq!(consumed, 1);
    assert_eq!(phase, 0);
}

#[test]
fn playlist_step_stays_inside_the_list() {
    assert_eq!(fvid_media::playlist_step(3, 0, -1), None);
    assert_eq!(fvid_media::playlist_step(3, 0, 1), Some(1));
    assert_eq!(fvid_media::playlist_step(3, 2, 1), None);
    assert_eq!(fvid_media::playlist_step(0, 0, 1), None);
}

#[test]
fn snapshot_bmp_is_bottom_up_bgr() {
    let bytes = fvid_media::encode_bmp(1, 1, &[0x00FF_0000]).unwrap();
    assert_eq!(&bytes[0..2], b"BM");
    assert_eq!(bytes[54], 0);
    assert_eq!(bytes[55], 0);
    assert_eq!(bytes[56], 255);
    let path = fvid_media::snapshot_path(Path::new("clips/demo.mp4"), 2);
    assert_eq!(path, PathBuf::from("clips/demo-fvid-2.bmp"));
}

#[test]
fn subtitle_text_and_track_cycle() {
    assert_eq!(fvid_media::plain_subtitle("Hello"), "Hello");
    assert_eq!(
        fvid_media::plain_subtitle(r"0,0,Default,,0,0,0,,{\i1}Hello{\i0}\Nthere"),
        "Hello\nthere"
    );
    let (start, end) = fvid_media::subtitle_window(1_000_000, 200, 1200);
    assert_eq!((start, end), (1_200_000, 2_200_000));
    let cues = [fvid_media::SubtitleCue {
        start_us: start,
        end_us: end,
        text: "Hello".into(),
    }];
    assert_eq!(fvid_media::active_subtitle(&cues, 1_200_000), Some("Hello"));
    assert_eq!(fvid_media::active_subtitle(&cues, 2_200_000), None);
    assert_eq!(fvid_media::cycle_track(2, 0, 1, false), 1);
    assert_eq!(fvid_media::cycle_track(2, 1, 1, false), 0);
    assert_eq!(fvid_media::cycle_track(2, 0, 1, true), 1);
    assert_eq!(fvid_media::cycle_track(2, 1, 1, true), -1);
    assert_eq!(fvid_media::cycle_track(2, -1, 1, true), 0);
}

#[test]
fn external_subtitles_and_device_name() {
    let cues = fvid_media::parse_srt(
        "1\n00:00:01,200 --> 00:00:02,000\nHello from file\n\n2\n00:00:02,000 --> 00:00:03,000\nNext\n",
    );
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].start_us, 1_200_000);
    assert_eq!(cues[0].text, "Hello from file");
    assert_eq!(
        fvid_media::parse_subtitle_clock("0:00:01.20"),
        Some(1_200_000)
    );
    let ass = fvid_media::parse_subtitle_text(
        "[Events]\nDialogue: 0,0:00:01.20,0:00:02.00,Default,,0,0,0,,Hello\n",
    )
    .unwrap();
    assert_eq!(ass[0].text, "Hello");
    let names = vec!["Speakers".into(), "HDMI".into()];
    assert_eq!(fvid_media::find_audio_device(&names, "hdmi"), Some("HDMI"));
    assert_eq!(fvid_media::find_audio_device(&names, " missing "), None);
    assert!(fvid_media::is_playback_url("https://example.test/a.mp4"));
    assert!(fvid_media::is_playback_url("rtsp://cam.example/stream"));
    assert!(fvid_media::is_playback_url("udp://239.1.1.1:5000"));
    assert!(!fvid_media::is_playback_url("file-not-a-url"));
    assert!(!fvid_media::is_playback_url("javascript://alert"));
    let a = fvid_media::ab_mark(None, 1_500_000).unwrap();
    assert_eq!(a.a_us, 1_500_000);
    assert!(a.b_us < 0);
    assert_eq!(fvid_media::ab_restart_us(a, 9_000_000), None);
    let both = fvid_media::ab_mark(Some(a), 4_000_000).unwrap();
    assert_eq!(both.a_us, 1_500_000);
    assert_eq!(both.b_us, 4_000_000);
    assert_eq!(fvid_media::ab_restart_us(both, 3_999_999), None);
    assert_eq!(fvid_media::ab_restart_us(both, 4_000_000), Some(1_500_000));
    let swapped = fvid_media::ab_mark(Some(a), 200_000).unwrap();
    assert_eq!((swapped.a_us, swapped.b_us), (200_000, 1_500_000));
    assert_eq!(fvid_media::ab_mark(Some(both), 0), None);
    let chapters = [0, 10_000_000, 20_000_000];
    assert_eq!(
        fvid_media::chapter_step(&chapters, 12_000_000, 1),
        Some(20_000_000)
    );
    assert_eq!(
        fvid_media::chapter_step(&chapters, 15_000_000, -1),
        Some(10_000_000)
    );
    assert_eq!(fvid_media::chapter_step(&chapters, 11_000_000, -1), Some(0));
    assert_eq!(fvid_media::chapter_step(&chapters, 25_000_000, 1), None);
    assert_eq!(
        fvid_media::cycle_repeat(fvid_media::RepeatMode::Off),
        fvid_media::RepeatMode::All
    );
    assert_eq!(
        fvid_media::playback_continue(3, 2, fvid_media::RepeatMode::Off),
        fvid_media::PlaybackContinue::Stop
    );
    assert_eq!(
        fvid_media::playback_continue(3, 2, fvid_media::RepeatMode::All),
        fvid_media::PlaybackContinue::Next(0)
    );
    assert_eq!(
        fvid_media::playback_continue(3, 1, fvid_media::RepeatMode::One),
        fvid_media::PlaybackContinue::Restart
    );
    assert_eq!(fvid_media::subtitle_delay_us(0, 2), 100_000);
    assert_eq!(fvid_media::subtitle_clock_us(1_000_000, 100_000), 900_000);
    assert_eq!(fvid_media::audio_delay_frames(50_000, 48_000), 2_400);
    assert_eq!(fvid_media::audio_delay_frames(-50_000, 48_000), -2_400);
    assert_eq!(fvid_media::audio_delay_frames(50_000, 0), 0);
    assert_eq!(fvid_media::step_audio_skew(3, 1, 10), (2, true, 0));
    assert_eq!(fvid_media::step_audio_skew(-3, 1, 10), (-2, false, 1));
    assert_eq!(fvid_media::step_audio_skew(-3, 1, 0), (-3, false, 0));
    assert_eq!(
        fvid_media::cycle_aspect(fvid_media::AspectMode::Source),
        fvid_media::AspectMode::Square
    );
    assert_eq!(
        fvid_media::cycle_aspect(fvid_media::AspectMode::FiveFour),
        fvid_media::AspectMode::Source
    );
    assert_eq!(
        fvid_media::frame_aspect(320, 240, fvid_media::AspectMode::Source),
        (320, 240)
    );
    assert_eq!(
        fvid_media::frame_aspect(320, 240, fvid_media::AspectMode::SixteenNine),
        (16, 9)
    );
    assert_eq!(fvid_media::fit_aspect(1000, 1000, 16, 9), (1000, 562));
    assert_eq!(fvid_media::fit_aspect(1920, 1080, 16, 9), (1920, 1080));
    assert_eq!(fvid_media::clamp_volume_milli(2_500), 2_000);
    assert_eq!(fvid_media::clamp_volume_milli(-5), 0);
    assert_eq!(fvid_media::clamp_volume_milli(1_500), 1_500);
    assert_eq!(
        fvid_media::center_crop(1920, 1080, 4, 3),
        (240, 0, 1440, 1080)
    );
    assert_eq!(fvid_media::center_crop(320, 240, 16, 9), (0, 30, 320, 180));
    assert_eq!(
        fvid_media::display_ratio(
            320,
            240,
            fvid_media::AspectMode::Source,
            fvid_media::AspectMode::SixteenNine
        ),
        (16, 9)
    );
    assert_eq!(
        fvid_media::display_ratio(
            320,
            240,
            fvid_media::AspectMode::Square,
            fvid_media::AspectMode::SixteenNine
        ),
        (1, 1)
    );
    assert_eq!(fvid_media::zoom_step(1_000, true), 2_000);
    assert_eq!(fvid_media::zoom_step(1_000, false), 500);
    assert_eq!(fvid_media::zoom_step(250, false), 250);
    assert_eq!(fvid_media::zoom_step(2_000, true), 2_000);
    assert_eq!(fvid_media::zoom_size(1920, 1080, 1_000), (1920, 1080));
    assert_eq!(fvid_media::zoom_size(1920, 1080, 2_000), (3840, 2160));
    assert_eq!(fvid_media::zoom_label(500), "1:2");
    let mut marks = Vec::new();
    assert!(fvid_media::insert_bookmark(&mut marks, 2_000_000));
    assert!(fvid_media::insert_bookmark(&mut marks, 500_000));
    assert!(!fvid_media::insert_bookmark(&mut marks, 2_000_000));
    assert_eq!(marks[0].media_us, 500_000);
    assert_eq!(fvid_media::bookmark_step(&marks, 0, 1), Some(500_000));
    assert_eq!(
        fvid_media::bookmark_step(&marks, 500_000, 1),
        Some(2_000_000)
    );
    assert_eq!(fvid_media::bookmark_step(&marks, 2_000_000, 1), None);
    assert_eq!(
        fvid_media::bookmark_step(&marks, 2_000_000, -1),
        Some(500_000)
    );
    let order = fvid_media::shuffled_indices(5, 42);
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3, 4]);
    assert_eq!(fvid_media::shuffled_indices(5, 42), order);
    assert_eq!(
        fvid_media::order_step(&order, 0, 1, false),
        Some((order[1], 1))
    );
    assert_eq!(fvid_media::order_step(&[2, 0, 1], 2, 1, false), None);
    assert_eq!(fvid_media::order_step(&[2, 0, 1], 2, 1, true), Some((2, 0)));
    let base = std::path::Path::new(r"D:\lists");
    let items = fvid_media::parse_playlist_text(
        "#EXTM3U\n#EXTINF:1,A\na.mp4\nhttp://example.test/b.mp4\n",
        base,
    );
    assert_eq!(items.len(), 2);
    assert!(items[0].ends_with("a.mp4"));
    assert!(fvid_media::is_playback_url(items[1].to_str().unwrap()));
    let pls = fvid_media::parse_playlist_text(
        "[playlist]\nFile1=c.mp3\nTitle1=C\nNumberOfEntries=1\n",
        base,
    );
    assert!(pls[0].ends_with("c.mp3"));
    assert!(fvid_media::is_hls_playlist(
        "#EXTM3U\n#EXT-X-TARGETDURATION:1\nseg.ts\n"
    ));
    assert!(
        fvid_media::parse_playlist_text("#EXTM3U\n#EXT-X-TARGETDURATION:1\nseg.ts\n", base)
            .is_empty()
    );
    let cues = [fvid_media::SubtitleCue {
        start_us: 0,
        end_us: 500_000,
        text: "Hi".into(),
    }];
    assert_eq!(
        fvid_media::active_subtitle(&cues, fvid_media::subtitle_clock_us(600_000, 200_000)),
        Some("Hi")
    );
    let corpus = std::fs::read_to_string("docs/PLAYER_CORPUS.md").expect("player corpus");
    let names: Vec<_> = corpus
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("| ")?;
            let (index, rest) = rest.split_once(" | ")?;
            let name = rest.strip_suffix(" |")?;
            index.parse::<usize>().ok()?;
            Some(name.trim())
        })
        .filter(|name| !name.is_empty() && *name != "Name" && *name != "---")
        .collect();
    assert!(
        names.len() >= 500,
        "player corpus has {} entries; need at least 500 real products",
        names.len()
    );
    assert!(names.iter().any(|name| name.eq_ignore_ascii_case("VLC")));
    assert!(names.iter().any(|name| name.eq_ignore_ascii_case("mpv")));
    assert!(
        names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("PotPlayer"))
    );
    assert_eq!(fvid_media::clamp_adjust_milli(3_000), 2_000);
    assert_eq!(
        fvid_media::adjust_pixel(40, 80, 120, 1_000, 1_000, 1_000, 1_000),
        (40, 80, 120)
    );
    let bright = fvid_media::adjust_pixel(40, 80, 120, 1_500, 1_000, 1_000, 1_000);
    assert!(bright.0 > 40 && bright.1 > 80 && bright.2 > 120);
    let gray = fvid_media::adjust_pixel(200, 40, 40, 1_000, 1_000, 0, 1_000);
    assert!((gray.0 as i32 - gray.1 as i32).abs() < 8);
    assert!((gray.1 as i32 - gray.2 as i32).abs() < 8);
    let hue = fvid_media::adjust_pixel(200, 40, 40, 1_000, 1_000, 1_000, 1_500);
    assert_ne!(hue, (200, 40, 40));
    assert_eq!(
        fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), true, false),
        (0.9, 0.2, 0.1, 0.8)
    );
    assert_eq!(
        fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), false, true),
        (0.1, 0.8, 0.9, 0.2)
    );
    assert_eq!(
        fvid_media::flip_uv((0.1, 0.2, 0.9, 0.8), true, true),
        (0.9, 0.8, 0.1, 0.2)
    );
    assert_eq!(
        fvid_media::cycle_rotate(fvid_media::RotateMode::Deg0),
        fvid_media::RotateMode::Deg90
    );
    assert_eq!(
        fvid_media::rotate_size(320, 240, fvid_media::RotateMode::Deg90),
        (240, 320)
    );
    assert_eq!(
        fvid_media::rotate_pixel(0, 0, 320, 240, fvid_media::RotateMode::Deg90),
        (0, 319)
    );
    assert_eq!(
        fvid_media::rotate_pixel(319, 0, 320, 240, fvid_media::RotateMode::Deg90),
        (0, 0)
    );
    assert_eq!(
        fvid_media::rotate_pixel(0, 0, 320, 240, fvid_media::RotateMode::Deg180),
        (319, 239)
    );
    let mut tone = fvid_media::ToneState::default();
    let mut flat = 0.0;
    for _ in 0..64 {
        flat = fvid_media::tone_step(0.25, &mut tone, 1_000, 1_000, 1_000);
    }
    assert!((flat - 0.25).abs() < 0.05);
    let mut boosted = fvid_media::ToneState::default();
    let mut loud = 0.0;
    for _ in 0..64 {
        loud = fvid_media::tone_step(0.25, &mut boosted, 2_000, 1_000, 1_000);
    }
    assert!(loud > flat);
    assert_eq!(fvid_media::EQ_BAND_COUNT, 10);
    assert_eq!(fvid_media::EQ_BAND_HZ[0], 60);
    assert_eq!(fvid_media::EQ_BAND_HZ[9], 16_000);
    let mut geq = fvid_media::GraphicEqState::default();
    let unity = fvid_media::eq_unity_gains();
    let mut flat_eq = 0.0;
    for _ in 0..128 {
        flat_eq = fvid_media::graphic_eq_step(0.25, &mut geq, &unity);
    }
    assert!((flat_eq - 0.25).abs() < 0.05);
    let mut bass_boost = fvid_media::eq_unity_gains();
    bass_boost[0] = 2_000;
    let mut geq_boost = fvid_media::GraphicEqState::default();
    let mut loud_eq = 0.0;
    for _ in 0..128 {
        loud_eq = fvid_media::graphic_eq_step(0.25, &mut geq_boost, &bass_boost);
    }
    assert!(loud_eq > flat_eq);
    assert_eq!(
        fvid_media::average_rgb_pixel(0x00_ff_00_00, 0x00_00_00_00),
        0x00_7f_00_00
    );
    let mut field = vec![
        0x00_ff_00_00u32,
        0x00_00_00_ff,
        0x00_00_ff_00,
        0x00_ff_ff_00,
    ];
    fvid_media::deinterlace_blend_rgb(&mut field, 2, 2);
    assert_eq!(field[0], field[2]);
    assert_eq!(field[1], field[3]);
    assert_eq!(
        field[0],
        fvid_media::average_rgb_pixel(0x00_ff_00_00, 0x00_00_ff_00)
    );
    let devices = vec!["Speakers".into(), "HDMI".into(), "USB DAC".into()];
    assert_eq!(
        fvid_media::cycle_output_device(&devices, "HDMI", 1),
        Some("USB DAC")
    );
    assert_eq!(
        fvid_media::cycle_output_device(&devices, "USB DAC", 1),
        Some("Speakers")
    );
    assert_eq!(
        fvid_media::cycle_output_device(&devices, "missing", -1),
        Some("USB DAC")
    );
    assert_eq!(fvid_media::cycle_output_device(&[], "x", 1), None);
    let stats = fvid_media::PlayStats {
        presented_frames: 12,
        skipped_frames: 3,
        width: 640,
        height: 360,
        source_width: 1920,
        source_height: 1080,
        audio: true,
        sample_rate: 48_000,
        channels: 2,
    };
    let line = fvid_media::format_play_stats(&stats, 65_000_000, 120_000_000);
    assert!(line.contains("shown 12"));
    assert!(line.contains("drop 3"));
    assert!(line.contains("1920x1080→640x360"));
    assert!(line.contains("48000 Hz 2ch"));
    let mut frame = [0.25f32, -0.5];
    fvid_media::apply_audio_channel(&mut frame, fvid_media::AudioChannelMode::Mono);
    assert!((frame[0] - (-0.125)).abs() < 1e-6);
    assert_eq!(frame[0], frame[1]);
    let mut swapped = [0.25f32, -0.5];
    fvid_media::apply_audio_channel(&mut swapped, fvid_media::AudioChannelMode::Reverse);
    assert_eq!(swapped, [-0.5, 0.25]);
    assert_eq!(
        fvid_media::cycle_audio_channel(fvid_media::AudioChannelMode::Reverse),
        fvid_media::AudioChannelMode::Karaoke
    );
    assert_eq!(
        fvid_media::cycle_audio_channel(fvid_media::AudioChannelMode::Karaoke),
        fvid_media::AudioChannelMode::Stereo
    );
    assert_eq!(fvid_media::clamp_subtitle_margin(500), 400);
    assert_eq!(fvid_media::subtitle_margin_px(12, 40), 52);
    assert_eq!(fvid_media::subtitle_margin_px(12, -20), 0);
    let png = fvid_media::encode_png(1, 1, &[0x00_ff_00_00]).unwrap();
    assert_eq!(
        &png[0..8],
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    );
    assert_eq!(
        fvid_media::cycle_snapshot_format(fvid_media::SnapshotFormat::Bmp),
        fvid_media::SnapshotFormat::Png
    );
    assert_eq!(
        fvid_media::snapshot_path_with_ext(std::path::Path::new("a.mp4"), 1, "png")
            .file_name()
            .and_then(|n| n.to_str()),
        Some("a-fvid-1.png")
    );
    let mut palette = vec![0u8; 1024];
    palette[4..8].copy_from_slice(&[255, 0, 0, 255]);
    assert_eq!(fvid_media::palette_rgba_pixel(1, &palette), 0xff_ff_00_00);
    let pixels = fvid_media::pal8_to_rgba(2, 1, &[1, 0], 2, &palette).unwrap();
    assert_eq!(pixels[0], 0xff_ff_00_00);
    assert_eq!(pixels[1], 0);
    assert_eq!(
        fvid_media::blend_rgba_over_rgb(0x00_00_ff_00, 0x80_ff_00_00),
        0x00_80_7f_00
    );
    let mut frame = vec![0u32; 4];
    let plane = fvid_media::BitmapSubtitle {
        start_us: 0,
        end_us: 1_000_000,
        x: 1,
        y: 0,
        width: 1,
        height: 1,
        pixels: vec![0xff_00_00_ff],
    };
    fvid_media::blit_bitmap_subtitle(&mut frame, 2, 2, &plane);
    assert_eq!(frame[1], 0x00_00_00_ff);
    assert!(fvid_media::active_bitmap_subtitle(std::slice::from_ref(&plane), 10).is_some());
    assert!(
        fvid_media::active_bitmap_subtitle(std::slice::from_ref(&plane), 2_000_000).is_none()
    );
    assert_eq!(fvid_media::parse_play_clock("90"), Some(90_000_000));
    assert_eq!(fvid_media::parse_play_clock("1:30.0"), Some(90_000_000));
    assert_eq!(fvid_media::parse_play_clock("bad"), None);
    assert_eq!(fvid_media::EQ_PRESET_COUNT, 18);
    assert_eq!(
        fvid_media::eq_preset_gains(fvid_media::EqPreset::Flat),
        fvid_media::eq_unity_gains()
    );
    let rock = fvid_media::eq_preset_db(fvid_media::EqPreset::Rock);
    assert!((rock[0] - 8.0).abs() < 0.01);
    assert!(fvid_media::eq_db_to_milli(0.0) == 1_000);
    assert!(fvid_media::eq_db_to_milli(-6.0) < 1_000);
    assert!(fvid_media::eq_db_to_milli(6.0) > 1_000);
    assert_eq!(
        fvid_media::cycle_eq_preset(fvid_media::EqPreset::Techno),
        fvid_media::EqPreset::Flat
    );
    assert_eq!(fvid_media::seek_step_us(false), fvid_media::SEEK_COARSE_US);
    assert_eq!(fvid_media::seek_step_us(true), fvid_media::SEEK_FINE_US);
    assert_eq!(
        fvid_media::media_display_title(
            std::path::Path::new("clips/demo.mp4"),
            Some(" My Title ")
        ),
        "My Title"
    );
    assert_eq!(
        fvid_media::media_display_title(std::path::Path::new("clips/demo.mp4"), None),
        "demo"
    );
    assert_eq!(
        fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Off),
        fvid_media::DeinterlaceMode::Blend
    );
    assert_eq!(
        fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Blend),
        fvid_media::DeinterlaceMode::Bob
    );
    assert_eq!(
        fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Bob),
        fvid_media::DeinterlaceMode::Linear
    );
    assert_eq!(
        fvid_media::cycle_deinterlace(fvid_media::DeinterlaceMode::Mean),
        fvid_media::DeinterlaceMode::Off
    );
    assert_eq!(
        fvid_media::deinterlace_label(fvid_media::DeinterlaceMode::Bob),
        "Bob"
    );
    let mut bob = vec![
        0x00_ff_00_00u32,
        0x00_00_00_ff,
        0x00_00_ff_00,
        0x00_ff_ff_00,
    ];
    fvid_media::apply_deinterlace_rgb(&mut bob, 2, 2, fvid_media::DeinterlaceMode::Bob);
    assert_eq!(bob[0], bob[2]);
    assert_eq!(bob[1], bob[3]);
    let mut unchanged = vec![1u32, 2, 3, 4];
    fvid_media::apply_deinterlace_rgb(&mut unchanged, 2, 2, fvid_media::DeinterlaceMode::Off);
    assert_eq!(unchanged, vec![1, 2, 3, 4]);
    assert_eq!(fvid_media::format_volume_osd(1_000, false), "Volume 100%");
    assert_eq!(fvid_media::format_volume_osd(1_500, false), "Volume 150%");
    assert_eq!(fvid_media::format_volume_osd(500, true), "Volume muted");
    assert_eq!(
        fvid_media::position_us_from_digit(5, 100_000_000),
        Some(50_000_000)
    );
    assert_eq!(
        fvid_media::position_us_from_digit(0, 100_000_000),
        Some(100_000_000)
    );
    assert_eq!(fvid_media::position_us_from_digit(3, -1), None);
    assert_eq!(
        fvid_media::media_us_from_fraction(0.25, 80_000_000),
        20_000_000
    );
    assert!((fvid_media::media_fraction(25_000_000, 100_000_000) - 0.25).abs() < f32::EPSILON);
    assert_eq!(
        fvid_media::volume_step_milli(1_000, fvid_media::VOLUME_STEP_MILLI),
        1_050
    );
    assert_eq!(
        fvid_media::volume_step_milli(1_990, 50),
        fvid_media::VOLUME_MAX_MILLI
    );
    assert_eq!(
        fvid_media::rate_step_milli(1_000, fvid_media::RATE_STEP_MILLI),
        1_100
    );
    assert_eq!(fvid_media::rate_step_milli(250, -100), 250);
    assert_eq!(fvid_media::format_play_clock(90_000_000), "01:30");
    assert_eq!(fvid_media::format_rate_osd(1_500), "1.50x");
    assert_eq!(
        fvid_media::frame_step_target_us(1_000_000, 40_000, fvid_media::FrameStep::Forward),
        1_040_000
    );
    assert_eq!(
        fvid_media::frame_step_target_us(30_000, 40_000, fvid_media::FrameStep::Backward),
        0
    );
    assert_eq!(
        fvid_media::clamp_balance_milli(3_000),
        fvid_media::BALANCE_MAX_MILLI
    );
    assert_eq!(
        fvid_media::balance_step_milli(
            fvid_media::BALANCE_CENTER_MILLI,
            -fvid_media::BALANCE_STEP_MILLI
        ),
        900
    );
    let mut stereo = [0.5f32, 0.5];
    fvid_media::apply_audio_balance(&mut stereo, fvid_media::BALANCE_CENTER_MILLI);
    assert!((stereo[0] - 0.5).abs() < f32::EPSILON && (stereo[1] - 0.5).abs() < f32::EPSILON);
    let mut left = [0.5f32, 0.5];
    fvid_media::apply_audio_balance(&mut left, fvid_media::BALANCE_MIN_MILLI);
    assert!((left[0] - 0.5).abs() < f32::EPSILON && left[1].abs() < f32::EPSILON);
    let mut mono = [0.5f32];
    fvid_media::apply_audio_balance(&mut mono, 0);
    assert!((mono[0] - 0.5).abs() < f32::EPSILON);
    assert_eq!(
        fvid_media::format_balance_osd(fvid_media::BALANCE_CENTER_MILLI),
        "Balance center"
    );
    assert_eq!(fvid_media::clamp_pan_px(0, 100, 100), 0);
    assert_eq!(fvid_media::clamp_pan_px(80, 100, 200), 50);
    assert_eq!(fvid_media::pan_step_px(0, 10, 100, 200), 10);
    let (x, y, w, h) = fvid_media::zoom_pan_rect((0.0, 0.0, 100.0, 100.0), 200.0, 200.0, 0, 0);
    assert!((w - 200.0).abs() < f32::EPSILON && (h - 200.0).abs() < f32::EPSILON);
    assert!((x - (-50.0)).abs() < f32::EPSILON && (y - (-50.0)).abs() < f32::EPSILON);
    assert_eq!(
        fvid_media::clamp_subtitle_scale_milli(3_000),
        fvid_media::SUBTITLE_SCALE_MAX_MILLI
    );
    assert!((fvid_media::subtitle_font_px(22.0, 2_000) - 44.0).abs() < f32::EPSILON);
    assert!(!fvid_media::cycle_eq_bypass(true));
    assert_eq!(fvid_media::format_eq_bypass_osd(false), "EQ on");
    assert_eq!(fvid_media::reset_av_delays(), (0, 0));
    assert_eq!(
        fvid_media::format_delay_osd("audio", 100_000),
        "audio delay 100 ms"
    );
    assert_eq!(fvid_media::volume_from_wheel(1_000, 1), 1_025);
    assert_eq!(fvid_media::volume_from_wheel(0, -1), 0);
    assert_eq!(fvid_media::clamp_seek_us(50, 40), 40);
    assert_eq!(fvid_media::format_ab_osd(None), "A-B off");
    assert_eq!(
        fvid_media::format_repeat_osd(fvid_media::RepeatMode::One),
        "repeat one"
    );
    assert_eq!(fvid_media::format_shuffle_osd(true), "shuffle on");
    assert_eq!(
        fvid_media::format_subtitle_scale_osd(1_500),
        "Subtitles 150%"
    );
    assert_eq!(fvid_media::format_jump_osd(90_000_000), "Jump 01:30");
    assert!(
        fvid_media::format_window_title("demo", 0, 60_000_000, true, 1_500).contains("1.50x")
    );
    assert_eq!(
        fvid_media::reset_tone_gains(),
        (
            fvid_media::TONE_UNITY_MILLI,
            fvid_media::TONE_UNITY_MILLI,
            fvid_media::TONE_UNITY_MILLI
        )
    );
    assert_eq!(
        fvid_media::tone_gain_step_milli(1_000, fvid_media::TONE_STEP_MILLI),
        1_100
    );
    let mut states = [fvid_media::ToneState::default()];
    let mut frame = [0.25f32];
    fvid_media::apply_tone_frame(&mut frame, &mut states, 2_000, 1_000, 1_000);
    assert!(frame[0] > 0.25);
    assert!(fvid_media::format_tone_osd(1_000, 1_000, 1_000).contains("Tone"));
    let opts = fvid_media::PlayRenderOptions::default();
    let src = vec![
        0x00_ff_00_00u32,
        0x00_00_ff_00,
        0x00_00_00_ff,
        0x00_ff_ff_00,
    ];
    let (w, h, out) = fvid_media::render_play_pixels(2, 2, &src, &opts, None);
    assert_eq!((w, h), (2, 2));
    assert_eq!(out.len(), 4);
    let mut rotated = opts.clone();
    rotated.rotate = fvid_media::RotateMode::Deg90;
    let (rw, rh, _) = fvid_media::render_play_pixels(2, 2, &src, &rotated, None);
    assert_eq!((rw, rh), (2, 2));
    assert_eq!(
        fvid_media::reset_video_adjust(),
        (1_000, 1_000, 1_000, 1_000, 1_000)
    );
    assert_eq!(fvid_media::adjust_step_milli(1_000, 100), 1_100);
    assert_eq!(fvid_media::reset_zoom_pan(), (1_000, 0, 0));
    assert!(fvid_media::format_zoom_osd(2_000).contains("2:1"));
    assert_eq!(
        fvid_media::eq_band_step_milli(1_000, 100),
        fvid_media::clamp_eq_milli(1_100)
    );
    assert_eq!(
        fvid_media::set_eq_gains_from_preset(fvid_media::EqPreset::Flat),
        fvid_media::eq_unity_gains()
    );
    assert!(fvid_media::format_eq_preset_osd(fvid_media::EqPreset::Rock).contains("Rock"));
    assert_eq!(fvid_media::format_pan_osd(10, -5), "Pan 10,-5");
    assert_eq!(
        fvid_media::initial_seek_us("1:30", 200_000_000),
        Some(90_000_000)
    );
    assert_eq!(
        fvid_media::initial_seek_us("1:30", 60_000_000),
        Some(60_000_000)
    );
    assert_eq!(fvid_media::initial_seek_us("bad", 60_000_000), None);
    assert_eq!(fvid_media::gamma_channel(128, 1_000), 128);
    assert!(fvid_media::gamma_channel(128, 2_000) > 128);
    assert_eq!(
        fvid_media::format_audio_channel_osd(fvid_media::AudioChannelMode::Mono),
        "Audio Mono"
    );
    let tmp = std::env::temp_dir().join("fvid-play-controls-playlist");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let a = tmp.join("a.mp4");
    std::fs::write(&a, b"x").unwrap();
    let list = tmp.join("list.m3u");
    std::fs::write(&list, format!("#EXTM3U\n{}\n", a.display())).unwrap();
    let expanded = fvid_media::expand_play_inputs(&[list]).unwrap();
    assert_eq!(expanded.len(), 1);
    assert!(expanded[0].ends_with("a.mp4"));
    assert_eq!(
        fvid_media::initial_stop_us("1:30", 200_000_000),
        Some(90_000_000)
    );
    assert!(fvid_media::should_stop_playback(
        90_000_001,
        Some(90_000_000)
    ));
    assert!(!fvid_media::should_stop_playback(10, Some(90_000_000)));
    assert_eq!(fvid_media::rate_from_wheel(1_000, 1), 1_050);
    assert_eq!(fvid_media::rate_from_wheel(250, -1), 250);
    assert_eq!(fvid_media::stop_playback_us(), 0);
    assert_eq!(fvid_media::format_stop_osd(), "Stopped");
    assert!(fvid_media::format_rotate_osd(fvid_media::RotateMode::Deg90).contains("90"));
    assert_eq!(fvid_media::format_flip_osd(true, true), "Flip HV");
    assert_eq!(
        fvid_media::seek_end_us(100_000_000, Some(50_000_000)),
        50_000_000
    );
    assert_eq!(fvid_media::seek_end_us(100_000_000, None), 100_000_000);
    assert_eq!(
        fvid_media::chapter_index(&[0, 10_000_000, 20_000_000], 15_000_000),
        Some(1)
    );
    assert!(fvid_media::format_chapter_osd(1, 3, 10_000_000).contains("2/3"));
    assert_eq!(fvid_media::format_pause_osd(true), "Paused");
    let mut marks = Vec::new();
    assert!(fvid_media::insert_bookmark(&mut marks, 1_000_000));
    assert_eq!(
        fvid_media::format_bookmark_osd(1_000_000, marks.len(), true),
        "bookmark 00:01 (1)"
    );
    fvid_media::clear_bookmarks(&mut marks);
    assert!(marks.is_empty());
    assert_eq!(fvid_media::format_playlist_osd(0, 3), "1/3");
    let m3u = fvid_media::format_playlist_m3u(&[
        std::path::PathBuf::from("a.mp4"),
        std::path::PathBuf::from("b.mp4"),
    ]);
    assert!(m3u.starts_with("#EXTM3U\n"));
    assert!(m3u.contains("a.mp4\n"));
    assert!(m3u.contains("b.mp4\n"));
    assert_eq!(fvid_media::soft_clip_sample(0.5), 0.5);
    assert!(fvid_media::soft_clip_sample(2.0) < 1.0);
    assert!(fvid_media::soft_clip_sample(-2.0) > -1.0);
    assert_eq!(
        fvid_media::remaining_media_us(30_000_000, 90_000_000),
        60_000_000
    );
    assert_eq!(
        fvid_media::cycle_position_display(fvid_media::PositionDisplay::Elapsed),
        fvid_media::PositionDisplay::Remaining
    );
    assert!(
        fvid_media::format_position_osd(
            30_000_000,
            90_000_000,
            fvid_media::PositionDisplay::Remaining
        )
        .starts_with('-')
    );
    assert!(
        fvid_media::format_position_osd(
            30_000_000,
            90_000_000,
            fvid_media::PositionDisplay::Both
        )
        .contains("(-")
    );
    let peak = fvid_media::normalizer_peak_step(0.1, 0.8, 1.0, 0.0);
    assert!((peak - 0.8).abs() < 0.01);
    assert!(fvid_media::normalizer_gain_milli(0.5, 0.95) > 1_000);
    assert!(fvid_media::apply_normalizer_sample(0.5, 2_000).abs() <= 1.0);
    assert!(fvid_media::format_normalizer_osd(true, 1_500).contains("1.50"));
    let mut wide = [0.5f32, -0.5];
    fvid_media::apply_stereo_width(&mut wide, 2_000);
    assert!((wide[0] - 1.0).abs() < 0.01);
    assert!((wide[1] + 1.0).abs() < 0.01);
    let mut mono = [0.5f32, -0.5];
    fvid_media::apply_stereo_width(&mut mono, 0);
    assert!((mono[0] - mono[1]).abs() < 0.01);
    assert_eq!(fvid_media::width_step_milli(1_000, 100), 1_100);
    assert!(fvid_media::format_width_osd(1_500).contains("1.50"));
    assert!((fvid_media::compress_sample(0.2, 0.35, 4.0) - 0.2).abs() < 0.001);
    assert!(fvid_media::compress_sample(0.9, 0.35, 4.0).abs() < 0.9);
    let mut loud = [0.9f32, -0.9];
    fvid_media::apply_compressor(&mut loud, true, 0.35, 4.0);
    assert!(loud[0].abs() < 0.9);
    assert_eq!(fvid_media::format_compressor_osd(true), "Compressor on");
    assert_eq!(fvid_media::cycle_sleep_timer_min(0), 15);
    assert_eq!(fvid_media::cycle_sleep_timer_min(120), 0);
    assert_eq!(fvid_media::sleep_deadline_secs(0, 100), None);
    assert_eq!(fvid_media::sleep_deadline_secs(15, 100), Some(1_000));
    assert!(fvid_media::sleep_timer_fired(Some(100), 100));
    assert!(!fvid_media::sleep_timer_fired(Some(101), 100));
    assert_eq!(fvid_media::format_sleep_osd(30), "Sleep in 30 min");
    assert_eq!(fvid_media::format_sleep_osd(0), "Sleep timer off");
    let mut phones = [1.0f32, 0.0];
    fvid_media::apply_crossfeed(&mut phones, 1_000);
    assert!(phones[0] < 1.0 && phones[1] > 0.0);
    assert_eq!(fvid_media::crossfeed_step_milli(0, 100), 100);
    assert_eq!(fvid_media::format_crossfeed_osd(0), "Crossfeed off");
    assert!(fvid_media::format_crossfeed_osd(500).contains("50%"));
    let mut marks = Vec::new();
    assert!(fvid_media::insert_bookmark(&mut marks, 1_500_000));
    assert!(fvid_media::insert_bookmark(&mut marks, 3_000_000));
    let exported = fvid_media::format_bookmarks_export(&marks);
    assert!(exported.contains("start-time=1.500"));
    assert!(exported.contains("start-time=3.000"));
    let parsed = fvid_media::parse_bookmarks_export(&exported);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].media_us, 1_500_000);
    assert_eq!(parsed[1].media_us, 3_000_000);
    assert_eq!(
        fvid_media::clamp_fov_milli(10_000),
        fvid_media::FOV_MIN_MILLI
    );
    assert_eq!(fvid_media::FOV_DEFAULT_MILLI, 80_000);
    let flat = vec![0x00_ff_00_00u32; 4 * 2];
    let view = fvid_media::project_equirect_view(
        4,
        2,
        &flat,
        2,
        2,
        0,
        0,
        fvid_media::FOV_DEFAULT_MILLI,
    );
    assert_eq!(view.len(), 4);
    assert!(fvid_media::format_spherical_osd(true, 45_000, 10_000, 80_000).contains("360°"));
    assert!(fvid_media::is_hdr_transfer(fvid_media::COLOR_TRC_SMPTE2084));
    assert!(fvid_media::is_hdr_transfer(fvid_media::COLOR_TRC_HLG));
    assert!(!fvid_media::is_hdr_transfer(1));
    assert_eq!(
        fvid_media::cycle_hdr_tonemap(fvid_media::HdrTonemap::Off),
        fvid_media::HdrTonemap::Clip
    );
    assert_eq!(
        fvid_media::auto_hdr_tonemap(fvid_media::COLOR_TRC_SMPTE2084),
        fvid_media::HdrTonemap::Hable
    );
    let mapped = fvid_media::apply_hdr_tonemap_pixel(
        200,
        200,
        200,
        fvid_media::HdrTonemap::Hable,
        fvid_media::COLOR_TRC_SMPTE2084,
    );
    assert!(mapped.0 <= 255);
    assert!(fvid_media::pq_eotf(0.5) > 0.0);
    assert!(fvid_media::hlg_eotf(0.5) > 0.0);
    assert!(fvid_media::pq_eotf(0.8) > fvid_media::pq_eotf(0.2));
    assert!((fvid_media::expand_hdr_channel(0.5, 1) - 1.25).abs() < 0.01);
    assert!(
        fvid_media::format_hdr_tonemap_osd(fvid_media::HdrTonemap::Hable).contains("hable")
    );
    let sbs = vec![
        0x00_ff_00_00u32,
        0x00_00_ff_00,
        0x00_ff_00_00,
        0x00_00_ff_00,
    ];
    let (aw, ah, anag) =
        fvid_media::apply_play_stereo3d(2, 2, &sbs, fvid_media::PlayStereo3D::SbslAnaglyph);
    assert_eq!((aw, ah), (1, 2));
    assert_eq!(anag.len(), 2);
    assert_eq!(
        fvid_media::cycle_play_stereo3d(fvid_media::PlayStereo3D::Off),
        fvid_media::PlayStereo3D::SbslAnaglyph
    );
    assert!(
        fvid_media::format_play_stereo3d_osd(fvid_media::PlayStereo3D::MonoLeft)
            .contains("mono-left")
    );
    assert_eq!(
        fvid_media::parse_hdr_tonemap("hable").unwrap(),
        fvid_media::HdrTonemap::Hable
    );
    assert_eq!(
        fvid_media::parse_play_stereo3d("sbsl").unwrap(),
        fvid_media::PlayStereo3D::SbslAnaglyph
    );
    assert_eq!(fvid_media::parse_degrees_milli("45.5").unwrap(), 45_500);
    let mut dual = vec![0u32; 64 * 32];
    for y in 0..32 {
        for x in 0..64 {
            dual[y * 64 + x] = if x < 32 { 0x00_ff_00_00 } else { 0x00_00_00_ff };
        }
    }
    let front = fvid_media::project_equirect_view(64, 32, &dual, 8, 8, 0, 0, 60_000);
    let back = fvid_media::project_equirect_view(64, 32, &dual, 8, 8, 180_000, 0, 60_000);
    assert_ne!(front[0], back[0]);
    let gray = vec![0x00_80_80_80u32; 4];
    let mut hdr_opts = fvid_media::PlayRenderOptions::default();
    hdr_opts.hdr_tonemap = fvid_media::HdrTonemap::Hable;
    hdr_opts.color_trc = fvid_media::COLOR_TRC_SMPTE2084;
    let flat = fvid_media::render_play_pixels(
        2,
        2,
        &gray,
        &fvid_media::PlayRenderOptions::default(),
        None,
    );
    let hdr = fvid_media::render_play_pixels(2, 2, &gray, &hdr_opts, None);
    assert_ne!(flat.2[0], hdr.2[0]);
    assert!(
        fvid_media::format_media_info_osd(
            "demo",
            1920,
            1080,
            60_000_000,
            fvid_media::COLOR_TRC_SMPTE2084,
            true
        )
        .contains("HDR PQ")
    );
    assert!(fvid_media::format_media_info_osd("demo", 640, 360, -1, 0, true).contains("360°"));
    let jump = fvid_media::random_seek_us(100_000_000, 42).unwrap();
    assert!(jump >= 0 && jump < 100_000_000);
    assert_eq!(fvid_media::random_seek_us(100_000_000, 42), Some(jump));
    assert_eq!(fvid_media::random_seek_us(0, 1), None);
    assert!(fvid_media::detect_equirect_aspect(3840, 1920));
    assert!(!fvid_media::detect_equirect_aspect(1920, 1080));
    assert_eq!(fvid_media::cycle_integer_zoom(1_000), 2_000);
    assert_eq!(fvid_media::cycle_integer_zoom(2_000), 250);
    assert_eq!(
        fvid_media::fit_window_to_video(640, 360, 1920, 1080),
        (640, 360)
    );
    assert_eq!(
        fvid_media::fit_window_to_video(3840, 2160, 1920, 1080),
        (1920, 1080)
    );
    assert_eq!(fvid_media::subtitle_opacity_u8(1_000), 255);
    assert_eq!(fvid_media::subtitle_opacity_u8(500), 127);
    assert!(fvid_media::format_subtitle_opacity_osd(800).contains("80%"));
    assert!(fvid_media::format_integer_zoom_osd(1_000).contains("1:1"));
    assert_eq!(fvid_media::format_track_osd("Audio", 1, 3), "Audio 2/3");
    assert_eq!(
        fvid_media::format_track_osd("Subtitles", -1, 2),
        "Subtitles off"
    );
    assert!(fvid_media::should_quit_at_end(
        true,
        fvid_media::PlaybackContinue::Stop
    ));
    assert!(!fvid_media::should_quit_at_end(
        true,
        fvid_media::PlaybackContinue::Next(1)
    ));
    assert_eq!(fvid_media::clamp_roll_milli(-5_000), 355_000);
    assert_eq!(
        fvid_media::roll_step_milli(0, fvid_media::ROLL_STEP_MILLI),
        5_000
    );
    assert!(fvid_media::format_spherical_osd_ex(true, 0, 0, 90_000, 80_000).contains("roll"));
    let level = fvid_media::project_equirect_view(64, 32, &dual, 8, 8, 0, 0, 60_000);
    let rolled =
        fvid_media::project_equirect_view_ex(64, 32, &dual, 8, 8, 0, 0, 90_000, 60_000);
    assert!(level.iter().zip(rolled.iter()).any(|(a, b)| a != b));
    assert_eq!(
        fvid_media::cycle_subtitle_position(fvid_media::SubtitlePosition::Bottom),
        fvid_media::SubtitlePosition::Center
    );
    assert_eq!(
        fvid_media::subtitle_block_top_y(
            100.0,
            2,
            10.0,
            8.0,
            fvid_media::SubtitlePosition::Top
        ),
        8.0
    );
    assert!(
        fvid_media::subtitle_block_top_y(
            100.0,
            2,
            10.0,
            8.0,
            fvid_media::SubtitlePosition::Bottom
        ) > 50.0
    );
    assert!(
        fvid_media::format_subtitle_position_osd(fvid_media::SubtitlePosition::Center)
            .contains("center")
    );
    assert!(!fvid_media::osd_should_clear(
        100,
        fvid_media::OSD_TIMEOUT_DEFAULT_MS
    ));
    assert!(fvid_media::osd_should_clear(
        3_000,
        fvid_media::OSD_TIMEOUT_DEFAULT_MS
    ));
    assert_eq!(
        fvid_media::clamp_osd_timeout_ms(10),
        fvid_media::OSD_TIMEOUT_MIN_MS
    );
    assert!(fvid_media::mouse_should_hide(
        1_000,
        fvid_media::MOUSE_HIDE_DEFAULT_MS
    ));
    assert!(!fvid_media::mouse_should_hide(
        10,
        fvid_media::MOUSE_HIDE_DEFAULT_MS
    ));
    assert_eq!(fvid_media::audio_peak_milli(&[0.0, 0.5, -0.25]), 500);
    assert_eq!(fvid_media::format_vu_osd(500), "VU 50%");
    let snap = fvid_media::snapshot_path_in_dir(
        Some(Path::new("shots")),
        Path::new("clips/demo.mp4"),
        3,
        "png",
    );
    assert_eq!(snap, PathBuf::from("shots/demo-fvid-3.png"));
    assert!(fvid_media::format_snapshot_dir_osd(Some(Path::new("shots"))).contains("shots"));
    assert_eq!(
        fvid_media::clamp_network_cache_ms(120_000),
        fvid_media::NETWORK_CACHE_MAX_MS
    );
    assert!(fvid_media::format_network_cache_osd(1_000).contains("1000"));
    assert!(fvid_media::format_hotkeys_help_osd().contains("Space"));
    assert_eq!(fvid_media::vu_bar_fills(0, 4), vec![0, 0, 0, 0]);
    assert_eq!(fvid_media::vu_bar_fills(1_000, 4), vec![100, 100, 100, 100]);
    let mid = fvid_media::vu_bar_fills(500, 4);
    assert_eq!(mid.len(), 4);
    assert!(mid[0] > 0 && mid[3] < 100);
    assert_eq!(fvid_media::cycle_rate_preset_milli(1_000), 500);
    assert_eq!(fvid_media::cycle_rate_preset_milli(500), 2_000);
    assert_eq!(fvid_media::cycle_rate_preset_milli(2_000), 1_000);
    assert_eq!(
        fvid_media::cycle_marquee_position(fvid_media::MarqueePosition::Top),
        fvid_media::MarqueePosition::Center
    );
    assert!(
        fvid_media::format_marquee_osd("Hello", fvid_media::MarqueePosition::Bottom)
            .contains("bottom")
    );
    assert_eq!(
        fvid_media::marquee_block_top_y(100.0, 20.0, 8.0, fvid_media::MarqueePosition::Top),
        8.0
    );
    assert!(fvid_media::format_title_osd("Demo Clip").contains("Demo"));
    assert!(fvid_media::should_drop_late_frame(
        fvid_media::DropFrameMode::Late,
        50_000,
        40_000
    ));
    assert!(!fvid_media::should_drop_late_frame(
        fvid_media::DropFrameMode::Off,
        50_000,
        40_000
    ));
    assert_eq!(
        fvid_media::cycle_drop_frame(fvid_media::DropFrameMode::Late),
        fvid_media::DropFrameMode::Off
    );
    assert_eq!(fvid_media::format_show_osd(true), "OSD on");
    assert!(!fvid_media::cycle_show_osd(true));
    assert!(
        fvid_media::PlayOptions {
            start_paused: true,
            ..fvid_media::PlayOptions::default()
        }
        .start_paused
    );
    assert_eq!(fvid_media::network_cache_delay_us(1_000), 1_000_000);
    assert_eq!(
        fvid_media::network_cache_delay_us(120_000),
        i64::from(fvid_media::NETWORK_CACHE_MAX_MS) * 1_000
    );
    assert_eq!(
        fvid_media::apply_display_effect_pixel(10, 20, 30, fvid_media::DisplayEffect::Invert),
        (245, 235, 225)
    );
    let (sr, sg, sb) =
        fvid_media::apply_display_effect_pixel(200, 100, 50, fvid_media::DisplayEffect::Sepia);
    assert!(sr >= sg && sg >= sb);
    assert_eq!(
        fvid_media::apply_display_effect_pixel(
            10,
            20,
            30,
            fvid_media::DisplayEffect::Grayscale
        )
        .0,
        fvid_media::apply_display_effect_pixel(
            10,
            20,
            30,
            fvid_media::DisplayEffect::Grayscale
        )
        .1
    );
    assert_eq!(
        fvid_media::prefer_track_index(&["eng", "rus", "jpn"], "ru", 0),
        1
    );
    assert_eq!(fvid_media::prefer_track_index(&["eng", "rus"], "de", 0), 0);
    assert!(fvid_media::controls_should_hide(
        4_000,
        fvid_media::CONTROLS_AUTOHIDE_DEFAULT_MS,
        true
    ));
    assert!(!fvid_media::controls_should_hide(4_000, 3_000, false));
    let resume = fvid_media::format_resume_positions(&[
        ("a.mp4".into(), 1_500_000),
        ("b.mkv".into(), 2_000_000),
    ]);
    let parsed = fvid_media::parse_resume_positions(&resume);
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        fvid_media::resume_seek_us(&parsed, "a.mp4"),
        Some(1_500_000)
    );
    assert!(fvid_media::http_should_reconnect(0, 3));
    assert!(!fvid_media::http_should_reconnect(3, 3));
    assert_eq!(
        fvid_media::scaletempo_duration_us(1_000_000, 2_000),
        500_000
    );
    assert_eq!(fvid_media::scale_hdr_display_channel(100, 100), 100);
    assert!(fvid_media::scale_hdr_display_channel(100, 400) > 100);
    assert_eq!(
        fvid_media::default_cache_ms(fvid_media::CacheDomain::File),
        300
    );
    assert!(fvid_media::format_cache_osd(fvid_media::CacheDomain::Live, 500).contains("live"));
    assert_eq!(
        fvid_media::secondary_subtitle_delay_us(0, 1),
        fvid_media::subtitle_delay_us(0, 1)
    );
    let prefixed = fvid_media::snapshot_path_with_prefix(
        Some(Path::new("out")),
        Path::new("clip.mp4"),
        "snap-",
        1,
        "png",
    );
    assert_eq!(prefixed, PathBuf::from("out/snap-clip-1.png"));
    assert!(fvid_media::format_snapshot_prefix_osd("snap-").contains("snap-"));
    assert_eq!(fvid_media::next_snapshot_index(3), 4);
    assert_eq!(fvid_media::apply_eq_preamp_sample(0.5, 1_000), 1.0);
    assert!(fvid_media::format_eq_preamp_osd(100).contains("preamp"));
    let mut frame = [0.5f32, -0.5];
    fvid_media::apply_spatializer(&mut frame, 1_000);
    assert_ne!(frame[0], 0.5);
    assert!(fvid_media::gapless_should_prefetch(10_000, 50_000));
    assert!(!fvid_media::gapless_should_prefetch(100_000, 50_000));
    assert_eq!(fvid_media::crossfade_gain_pair(500, 1_000), (0.5, 0.5));
    assert!(fvid_media::format_crossfade_osd(500).contains("500"));
    let rg = fvid_media::replaygain_milli_from_db_milli(0);
    assert_eq!(rg, fvid_media::REPLAYGAIN_UNITY_MILLI);
    assert!(fvid_media::buffer_health_pct(500, 1_000) == 50);
    assert!(fvid_media::format_buffer_health_osd(500, 1_000).contains("50%"));
    assert_eq!(
        fvid_media::snap_seek_to_keyframe(1_500_000, &[0, 1_000_000, 2_000_000]),
        1_000_000
    );
    let mut karaoke = [0.8f32, 0.2];
    fvid_media::apply_audio_channel(&mut karaoke, fvid_media::AudioChannelMode::Karaoke);
    assert!((karaoke[0] + karaoke[1]).abs() < 0.01);
    assert_eq!(
        fvid_media::subtitle_color_rgba(fvid_media::SubtitleColor::Yellow)[1],
        255
    );
    assert!(
        fvid_media::format_subtitle_color_osd(fvid_media::SubtitleColor::Cyan).contains("Cyan")
    );
    assert_eq!(
        fvid_media::prefer_forced_subtitle_index(&[false, true, false], 0),
        1
    );
    assert!(fvid_media::momentary_lufs_from_peak_milli(1_000) < 0);
    let spectrum = fvid_media::spectrum_bar_fills(&[0.0, 0.5, -0.25, 0.1], 4);
    assert_eq!(spectrum.len(), 4);
    assert!(spectrum.iter().any(|v| *v > 0));
    let mut paths = vec![PathBuf::from("b/z.mp4"), PathBuf::from("a/y.mp4")];
    fvid_media::sort_playlist_paths(&mut paths, fvid_media::PlaylistSort::Name);
    assert_eq!(paths[0].file_name().unwrap(), "y.mp4");
    assert!(fvid_media::format_bookmark_label(90_000_000, Some("Act 1")).contains("Act 1"));
    let mut recent = Vec::new();
    fvid_media::push_recent_path(&mut recent, PathBuf::from("one.mp4"));
    fvid_media::push_recent_path(&mut recent, PathBuf::from("two.mp4"));
    assert_eq!(recent[0], PathBuf::from("two.mp4"));
    assert!(fvid_media::format_recent_osd(&recent).contains("two.mp4"));
    assert_eq!(
        fvid_media::deinterlace_label(fvid_media::DeinterlaceMode::Linear),
        "Linear"
    );
    assert_eq!(
        fvid_media::seek_step_us_ex(false, fvid_media::SeekJump::default()),
        fvid_media::SEEK_COARSE_US
    );
    let jump = fvid_media::cycle_seek_jump(fvid_media::SeekJump::default());
    assert_eq!(jump.coarse_us, 30_000_000);
    assert_eq!(
        fvid_media::cycle_video_post_fx(fvid_media::VideoPostFx::Off),
        fvid_media::VideoPostFx::Blur
    );
    let mut sharp = vec![0u32; 9];
    sharp[4] = 0x00_80_80_80;
    fvid_media::apply_video_post_fx(&mut sharp, 3, 3, fvid_media::VideoPostFx::Sharpen);
    let mut blur = vec![0x00_ff_00_00u32, 0, 0, 0];
    fvid_media::apply_video_post_fx(&mut blur, 2, 2, fvid_media::VideoPostFx::Blur);
    assert_ne!(blur[0], 0x00_ff_00_00);
    let mut surround = [1.0f32, -1.0, 0.5, 0.0, 0.25, -0.25];
    fvid_media::downmix_surround_to_stereo(&mut surround, 6);
    assert!(surround[0].abs() <= 1.0);
    assert!(fvid_media::format_downmix_osd(true).contains("On"));
    assert!(fvid_media::format_scaletempo_osd(false).contains("Off"));
    assert!(fvid_media::format_minimal_interface_osd(true).contains("Minimal"));
    assert_eq!(
        fvid_media::apply_audio_pitch_sample_index(1_000, 2_000),
        500
    );
    assert!(fvid_media::format_audio_pitch_osd(1_000).contains("1.00"));
    assert_eq!(
        fvid_media::cycle_visualization(fvid_media::VisualizationMode::Off),
        fvid_media::VisualizationMode::Spectrum
    );
    let scope = fvid_media::scope_samples_u8(&[0.0, 0.5, -0.5, 0.25], 4);
    assert_eq!(scope.len(), 4);
    let bars = fvid_media::audio_bargraph_fills(&[500, 1_000, 1_500], 3);
    assert_eq!(bars.len(), 3);
    let mut cur = vec![0x00_ff_00_00u32; 4];
    let prev = vec![0x00_00_00_ffu32; 4];
    fvid_media::apply_motion_blur_rgb(&mut cur, 2, 2, &prev);
    assert_ne!(cur[0], 0x00_ff_00_00);
    assert_eq!(fvid_media::image_duration_us(10), 10_000_000);
    assert_eq!(
        fvid_media::cycle_video_post_fx(fvid_media::VideoPostFx::Grain),
        fvid_media::VideoPostFx::MotionBlur
    );
    assert_eq!(
        fvid_media::cycle_closed_caption(fvid_media::ClosedCaptionChannel::Off),
        fvid_media::ClosedCaptionChannel::Cc1
    );
    let crop = fvid_media::clamp_crop_pixels(
        fvid_media::CropPixels {
            left: 10,
            top: 20,
            right: 10,
            bottom: 20,
        },
        100,
        100,
    );
    assert_eq!(fvid_media::crop_output_size(100, 100, crop), (80, 60));
    assert_eq!(fvid_media::audio_desync_us(50), 50_000);
    assert_eq!(fvid_media::prefer_program_index(&[1, 2, 100], 100, 0), 2);
    assert_eq!(
        fvid_media::cycle_subtitle_encoding(fvid_media::SubtitleEncoding::Utf8),
        fvid_media::SubtitleEncoding::Cp1251
    );
    assert!(fvid_media::format_wallpaper_osd(true).contains("On"));
    assert_eq!(fvid_media::teletext_page_step(100, 1), 101);
    assert!(fvid_media::format_teletext_osd(888, true).contains("888"));
    assert_eq!(
        fvid_media::locked_window_size(1920, 1080, 800, 800, true),
        (800, 450)
    );
    assert_eq!(
        fvid_media::format_snapshot_sequential_name("shot-", 7, 3, "png"),
        "shot-007.png"
    );
    assert!(fvid_media::format_hw_decode_osd(false).contains("Off"));
    assert!(fvid_media::format_media_fingerprint_osd("abcdef0123456789").contains("…"));
    assert_eq!(
        fvid_media::logo_anchor_xy(200, 100, 40, 20, fvid_media::LogoPosition::TopRight, 4),
        (156, 4)
    );
    assert_eq!(
        fvid_media::mosaic_tile_rect(100, 100, 2, 2, 3),
        (50, 50, 50, 50)
    );
    assert!((fvid_media::apply_param_eq_sample(0.5, 1_000) - 1.0).abs() < 1e-6);
    assert!(fvid_media::apply_amplifier_sample(0.8, 2_000).abs() <= 1.0);
    let rec = fvid_media::format_record_path(Some(Path::new("out")), "cap", 2, "mkv");
    assert_eq!(rec, PathBuf::from("out/cap-rec-2.mkv"));
    assert!(fvid_media::format_record_osd(true, Some(Path::new("a.mkv"))).contains("a.mkv"));
    assert_eq!(
        fvid_media::cycle_proxy_mode(fvid_media::ProxyMode::Off),
        fvid_media::ProxyMode::Http
    );
    assert_eq!(
        fvid_media::prefer_stream_quality_index(&[400, 800, 1600], 900),
        1
    );
    let media = Path::new("movie.mp4");
    let subs = [
        Path::new("other.srt"),
        Path::new("movie.en.srt"),
        Path::new("movie.srt"),
    ];
    assert_eq!(
        fvid_media::prefer_external_subtitle_path(media, &subs),
        Some(PathBuf::from("movie.srt"))
    );
    assert_eq!(
        fvid_media::cycle_spherical_projection(fvid_media::SphericalProjection::Equirect),
        fvid_media::SphericalProjection::DualFisheye
    );
    let planet = fvid_media::project_little_planet(8, 4, &vec![0x00_80_80_80u32; 32], 4, 4, 0);
    assert_eq!(planet.len(), 16);
    let cube = fvid_media::sample_cubemap_pixel(&[0x00_ff_00_00u32; 96], 24, 4, 1.0, 0.0, 0.0);
    assert_eq!(cube, 0x00_ff_00_00);
    assert!(
        fvid_media::format_hdr_metadata_osd(1_000, 400, fvid_media::COLOR_TRC_SMPTE2084)
            .contains("MaxCLL")
    );
    assert_eq!(
        fvid_media::color_primaries_label(fvid_media::COLOR_PRIMARIES_BT2020),
        "BT.2020"
    );
    assert_eq!(
        fvid_media::silence_skip_target_us(
            1_000_000,
            10_000_000,
            &[(500_000, 2_000_000)],
            100_000
        ),
        Some(2_000_000)
    );
    assert_eq!(fvid_media::cycle_video_track(3, 2), 0);
    assert_eq!(
        fvid_media::prefer_hearing_impaired_subtitle_index(&[false, true], 0),
        1
    );
    assert_eq!(
        fvid_media::skip_marker_target_us(10, Some(90), Some(1_000)),
        Some(90)
    );
    let mut hdr_opts = fvid_media::PlayRenderOptions::default();
    hdr_opts.spherical = true;
    hdr_opts.spherical_projection = fvid_media::SphericalProjection::LittlePlanet;
    let projected =
        fvid_media::render_play_pixels(8, 4, &vec![0x00_40_40_40u32; 32], &hdr_opts, None);
    assert_eq!(projected.0, 8);
    let filtered = fvid_media::filter_playlist_paths(
        &[PathBuf::from("a/foo.mp4"), PathBuf::from("b/bar.mkv")],
        "foo",
    );
    assert_eq!(filtered.len(), 1);
    let mut favs = Vec::new();
    assert!(fvid_media::toggle_favorite(
        &mut favs,
        PathBuf::from("a.mp4")
    ));
    assert!(!fvid_media::toggle_favorite(
        &mut favs,
        PathBuf::from("a.mp4")
    ));
    assert!(fvid_media::should_toggle_fullscreen_on_click(2, true));
    assert_eq!(fvid_media::scrub_preview_us(0.5, 100_000_000), 50_000_000);
    assert!(fvid_media::apply_night_mode_sample(0.9, true).abs() < 0.9);
    assert!(fvid_media::format_remote_control_osd(true, 8080).contains("8080"));
    assert!(fvid_media::format_bitperfect_osd(false).contains("Off"));
    assert_eq!(
        fvid_media::frame_rate_milli_from_duration_us(16_667),
        59_998
    );
    assert_eq!(
        fvid_media::seek_from_wheel(1_000_000, 1, 5_000_000),
        6_000_000
    );
    let mut queue = vec![2usize];
    fvid_media::queue_insert(&mut queue, 5, true);
    assert_eq!(queue[0], 5);
    assert_eq!(
        fvid_media::chapter_thumbnail_times(&[], 10_000_000, 5).len(),
        5
    );
    assert!(fvid_media::format_forced_only_osd(true).contains("Forced"));
    assert_eq!(
        fvid_media::parse_spherical_projection("cubemap").unwrap(),
        fvid_media::SphericalProjection::Cubemap
    );
    assert!(fvid_media::cardboard_eye_yaw_offset_milli(63_000, 90_000) > 0);
    assert_eq!(
        fvid_media::cycle_vr_display(fvid_media::VrDisplayMode::Off),
        fvid_media::VrDisplayMode::Cardboard
    );
    assert_eq!(
        fvid_media::parse_webvtt_timestamp("01:02.500"),
        Some(62_500_000)
    );
    assert_eq!(fvid_media::format_webvtt_timestamp(62_500_000), "01:02.500");
    assert_eq!(
        fvid_media::cycle_ambisonic(fvid_media::AmbisonicMode::Off),
        fvid_media::AmbisonicMode::FirstOrder
    );
    assert!(
        fvid_media::format_cast_osd(fvid_media::CastProtocol::AirPlay, "TV").contains("TV")
    );
    let lib = fvid_media::media_library_entries(
        Path::new("/media"),
        &["a.mp4", "readme.txt", "b.flac"],
    );
    assert_eq!(lib.len(), 2);
    let clip = fvid_media::parse_smil_clip_line("src=\"ep.mp4\" begin=12.5").unwrap();
    assert_eq!(clip.begin_us, 12_500_000);
    assert!(fvid_media::format_named_bookmark_osd("Act 1", 90_000_000).contains("Act 1"));
    assert!(fvid_media::format_hdr_mastering_osd(50, 1_000).contains("nits"));
    assert!(fvid_media::hdr_gamut_warning(
        fvid_media::COLOR_PRIMARIES_BT2020,
        true
    ));
    assert_eq!(
        fvid_media::cardboard_eye_rect(1920, 1080, 1),
        (960, 0, 960, 1080)
    );
    let left = fvid_media::cardboard_view_yaw_milli(0, 63_000, 90_000, true);
    let right = fvid_media::cardboard_view_yaw_milli(0, 63_000, 90_000, false);
    assert!(left != right);
    let lyrics = [
        fvid_media::LyricLine {
            start_us: 0,
            text: "one".into(),
        },
        fvid_media::LyricLine {
            start_us: 1_000_000,
            text: "two".into(),
        },
    ];
    assert_eq!(
        fvid_media::active_lyric_line(&lyrics, 1_500_000).map(|l| l.text.as_str()),
        Some("two")
    );
    let mut slots = [(None, None); 2];
    assert!(fvid_media::ab_slot_store(&mut slots, 0, 1_000, 2_000));
    assert_eq!(fvid_media::ab_slot_load(&slots, 0), Some((1_000, 2_000)));
    assert!(
        fvid_media::format_play_stats_csv(10, 1, 5_000_000, 1_000).contains("presented=10")
    );
    assert_eq!(
        fvid_media::parse_hdr_maxcll_maxfall("1000,400").unwrap(),
        (1_000, 400)
    );
    let _ = fvid_media::apply_deband_pixel(128, 128, 128, 500, 3, 5);
    assert!(fvid_media::format_deband_osd(200).contains("Deband"));
    assert!(fvid_media::format_dolby_vision_osd(Some(5)).contains("5"));
    assert_eq!(fvid_media::image_loop_remaining(3, 1), Some(2));
    let edl = fvid_media::parse_edl_line("clip.mp4 1.5 4.0").unwrap();
    assert_eq!(edl.in_us, 1_500_000);
    assert_eq!(
        fvid_media::pip_rect(1920, 1080, 320, 180, 16, true),
        (1584, 884, 320, 180)
    );
    assert_eq!(
        fvid_media::thumbnail_seek_us(2, 10, 100_000_000),
        20_000_000
    );
    assert_eq!(fvid_media::seek_from_drag_px(100, 50), 2_000_000);
    let box_ = fvid_media::clamp_crop_box_milli(fvid_media::CropBoxMilli {
        x0: 100,
        y0: 100,
        x1: 900,
        y1: 900,
    });
    let crop = fvid_media::crop_box_to_pixels(box_, 1000, 1000);
    assert_eq!(crop.left, 100);
    assert!(fvid_media::locked_pitch_milli(40_000, true, 0) == 0);
    let peak = fvid_media::estimate_frame_peak_milli(&[0x00_ff_ff_ffu32; 4], 1);
    assert_eq!(peak, 1_000);
    assert!(fvid_media::suggest_hdr_nits_from_peak(800, 100) >= 100);
    assert!(fvid_media::format_stream_rendition_osd(1920, 1080, 5_000_000).contains("1920"));
    assert_eq!(
        fvid_media::cycle_spherical_stereo(fvid_media::SphericalStereoLayout::Mono),
        fvid_media::SphericalStereoLayout::TopBottom
    );
    assert_eq!(
        fvid_media::spherical_stereo_uv_rect(
            fvid_media::SphericalStereoLayout::SideBySide,
            true
        ),
        (500, 0, 1_000, 1_000)
    );
    assert_eq!(fvid_media::recenter_spherical_view(), (0, 0, 0));
    assert_eq!(fvid_media::compass_heading_deg(90_000), 90);
    let (bu, bv) = fvid_media::barrel_distort_uv_milli(500, 500, 500);
    assert_eq!((bu, bv), (500, 500));
    assert_eq!(fvid_media::blend_tonemap_channel(200, 100, 500), 150);
    let desat = fvid_media::apply_hdr_highlight_desat_pixel(255, 200, 180, 500);
    assert!(desat.0 <= 255);
    let mastering = fvid_media::parse_hdr_mastering_nits("0.005,1000").unwrap();
    assert_eq!(mastering.0, 5);
    assert_eq!(mastering.1, 1_000);
    let wb = fvid_media::apply_white_balance_pixel(128, 128, 128, 3_200);
    assert_ne!(wb.0, wb.2);
    let mut letter = vec![0u32; 16];
    for y in 1..3 {
        for x in 1..3 {
            letter[y * 4 + x] = 0x00_ff_ff_ff;
        }
    }
    let bars = fvid_media::detect_letterbox_bars(4, 4, &letter, 16);
    assert!(bars.1 > 0 || bars.3 > 0 || bars.0 > 0);
    assert_eq!(
        fvid_media::playlist_edge_fade_gain_milli(0, 10_000_000, 1_000_000),
        0
    );
    assert_eq!(fvid_media::audio_duck_gain_milli(true, 400), 400);
    let wave = fvid_media::waveform_column_fills(2, 2, &[0x00_80_80_80u32; 4], 2);
    assert_eq!(wave.len(), 2);
    assert_eq!(
        fvid_media::parse_spherical_stereo("sbs").unwrap(),
        fvid_media::SphericalStereoLayout::SideBySide
    );
    assert!(fvid_media::hlg_ootf_channel(0.25, 1.2) > 0.0);
    assert_eq!(fvid_media::cycle_fov_preset_milli(90_000), 110_000);
    assert!(fvid_media::format_hdr_headroom_osd(400, 1_000).contains('+'));
    assert_eq!(
        fvid_media::format_timecode_osd(1_000_000, 24_000),
        "00:00:01:00"
    );
    assert_eq!(
        fvid_media::format_chapter_list_export(&[0, 1_000_000]),
        "00:00\n00:01"
    );
    let den = fvid_media::apply_box_denoise_pixel(2, 2, &[0x00_10_10_10u32; 4], 0, 0, 500);
    assert_ne!(den, 0);
    assert!(fvid_media::apply_dialogue_enhance_sample(0.5, 500).abs() > 0.5);
    let vs = fvid_media::vectorscope_quadrant_counts(&[0x00_ff_00_ffu32], 1);
    assert_eq!(vs.iter().sum::<u32>(), 1);
    assert_eq!(fvid_media::yaw_from_swipe_px(90, 1), 90_000);
    let (gy, _) = fvid_media::gyro_look_delta_milli(1_000, 0, 500);
    assert_eq!(gy, 500);
    assert!(fvid_media::vr_vignette_gain_milli(0, 0, 1_000) < 1_000);
    let (face, _, _) = fvid_media::eac_face_uv_from_dir(1.0, 0.0, 0.0);
    assert_eq!(face, 0);
    assert_eq!(fvid_media::apply_hdr_black_lift_channel(10, 100), 25);
    assert_eq!(
        fvid_media::parse_m3u_extinf_title("#EXTINF:123,Track Title").as_deref(),
        Some("Track Title")
    );
    assert_eq!(
        fvid_media::thumbnail_grid_rect(2, 2, 3, 100, 100),
        (50, 50, 50, 50)
    );
    assert_eq!(
        fvid_media::media_key_label(fvid_media::MediaKeyAction::Next),
        "Next"
    );
    assert!(fvid_media::format_icecast_metadata_osd(Some("A"), Some("T")).contains('—'));
    assert_eq!(fvid_media::timeshift_lag_us(100, 40), 60);
    assert_eq!(
        fvid_media::instant_replay_us(50_000_000, 10_000_000),
        40_000_000
    );
    assert_eq!(
        fvid_media::phase_correlation_milli(&[1.0, 0.5], &[1.0, 0.5]),
        1_000
    );
    assert_eq!(fvid_media::true_peak_milli(&[0.5, -0.25]), 500);
    assert!(fvid_media::format_atmos_layout_osd(7, 16).contains("objects"));
    assert_eq!(fvid_media::ass_override_margin_px(40, Some(80)), 80);
    assert_eq!(
        fvid_media::network_bandwidth_bps(1_250_000, 1_000),
        10_000_000
    );
    assert_eq!(fvid_media::multi_room_sync_target_us(100, 200, 0), 150);
    assert_eq!(fvid_media::hdr_sdr_ratio_milli(1_000, 100), 10_000);
    assert_eq!(fvid_media::accelerometer_horizon_pitch_milli(0), 0);
    let mut soft = fvid_media::PlayRenderOptions::default();
    soft.hdr_tonemap = fvid_media::HdrTonemap::Hable;
    soft.color_trc = fvid_media::COLOR_TRC_SMPTE2084;
    soft.tonemap_strength_milli = 500;
    soft.hdr_highlight_desat_milli = 400;
    soft.hdr_black_lift_milli = 50;
    soft.color_temp_kelvin = 4_000;
    let gray = vec![0x00_c0_c0_c0u32; 4];
    let full = fvid_media::render_play_pixels(
        2,
        2,
        &gray,
        &fvid_media::PlayRenderOptions {
            hdr_tonemap: fvid_media::HdrTonemap::Hable,
            color_trc: fvid_media::COLOR_TRC_SMPTE2084,
            ..fvid_media::PlayRenderOptions::default()
        },
        None,
    );
    let blended = fvid_media::render_play_pixels(2, 2, &gray, &soft, None);
    assert_ne!(full.2[0], blended.2[0]);
    assert_eq!(
        fvid_media::parse_spherical_projection("eac").unwrap(),
        fvid_media::SphericalProjection::Eac
    );
    let eac_px = fvid_media::sample_eac_pixel(&[0x00_11_22_33u32; 96], 96, 16, 1.0, 0.0, 0.0);
    assert_ne!(eac_px, 0xFFFF_FFFF);
    let (r, g, b) = fvid_media::chromatic_aberration_uv_milli(800, 500, 500);
    assert_ne!(r.0, b.0);
    assert_eq!(g, (800, 500));
    assert_eq!(
        fvid_media::cycle_ambisonic_order(fvid_media::AmbisonicChannelOrder::AcnSn3d),
        fvid_media::AmbisonicChannelOrder::AcnN3d
    );
    assert!(fvid_media::apply_soft_limiter_sample(1.5, 800).abs() <= 1.0);
    assert!(fvid_media::apply_echo_sample(0.5, 0.25, 400) > 0.5);
    let mut lp = 0.0f32;
    assert!(fvid_media::apply_lowpass_1pole(1.0, &mut lp, 200) > 0.0);
    let st = fvid_media::short_term_lufs_from_peaks(&[500, 600, 400]);
    assert!(st < 0);
    assert!(fvid_media::loudness_range_l_milli(&[-200, -150, -100, -50]) > 0);
    let dub = fvid_media::anaglyph_dubois(0x00_ff_00_00, 0x00_00_00_ff);
    assert_ne!(dub, 0);
    let mut logo = vec![0x00_80_80_80u32; 16];
    fvid_media::apply_delogo_rect(4, 4, &mut logo, 1, 1, 2, 2);
    assert_eq!(
        fvid_media::filter_playlist_by_extension(&["a.mp4".into(), "b.txt".into()], &["mp4"]),
        vec!["a.mp4".to_string()]
    );
    assert_eq!(
        fvid_media::detect_bpm_from_onset_gaps_ms(&[500, 500, 500]),
        120
    );
    assert_eq!(fvid_media::haas_delay_samples(48_000, 1_000), 48);
    assert_eq!(fvid_media::atempo_duration_us(2_000_000, 2_000), 1_000_000);
    assert!(fvid_media::apply_chorus_sample(0.5, 0.2, 500) != 0.5);
    assert!(fvid_media::apply_reverb_sample(0.4, 0.2, 0.1, 500) != 0.4);
    assert!(fvid_media::format_ass_force_style("Sans", 24, "").contains("FontSize=24"));
    let bt = fvid_media::apply_bt2446_tonemap_pixel(200, 200, 200, 1_000);
    assert!(bt.0 > 0);
    let hs = fvid_media::SphericalHotspot {
        yaw_deg_milli: 0,
        pitch_deg_milli: 0,
        radius_deg_milli: 5_000,
        label: "door".into(),
    };
    assert!(fvid_media::spherical_hotspot_hit(&hs, 1_000, 0));
    assert_eq!(
        fvid_media::parse_webvtt_region_id("REGION id:banner width:50%").as_deref(),
        Some("banner")
    );
    assert_eq!(
        fvid_media::thumbnail_cache_key("/a.mp4", 5_500_000, 1_000_000),
        "/a.mp4@5"
    );
    assert_eq!(
        fvid_media::integrated_lufs_from_short_term(&[-230, -220, -210]),
        -220
    );
    assert_eq!(
        fvid_media::parse_hdr_tonemap("mobius").unwrap(),
        fvid_media::HdrTonemap::Mobius
    );
    assert_eq!(
        fvid_media::parse_spherical_projection("panini").unwrap(),
        fvid_media::SphericalProjection::Panini
    );
    let pan = fvid_media::project_panini_view(
        4,
        2,
        &[0x00_10_20_30u32; 8],
        2,
        2,
        0,
        0,
        0,
        90_000,
        1_000,
    );
    assert_eq!(pan.len(), 4);
    let (cu, cv) = fvid_media::brown_conrady_uv_milli(500, 500, 0, 0);
    assert_eq!((cu, cv), (500, 500));
    assert!(fvid_media::ictcp_intensity_milli(200, 200, 200) > 0);
    assert_eq!(
        fvid_media::prefer_abr_rendition_index(&[500_000, 1_500_000, 3_000_000], 2_000_000),
        1
    );
    assert_eq!(
        fvid_media::storyboard_tile_index(25_000_000, 10_000_000, 10),
        2
    );
    assert_eq!(fvid_media::watch_progress_milli(30, 100), 300);
    assert!(fvid_media::up_next_should_start(5_000_000, 10_000_000));
    assert!(fvid_media::format_scrobble_line("A", "T", 180_000_000).contains("180"));
    assert!(fvid_media::gaze_dwell_triggered(true, 500, 400));
    assert_eq!(
        fvid_media::parse_ttml_clock("00:00:01.500"),
        Some(1_500_000)
    );
    assert_eq!(fvid_media::clamp_dvr_playhead_us(50, 100, 30), 70);
    let pq = fvid_media::pq_oetf(fvid_media::pq_eotf(0.5));
    assert!((pq - 0.5).abs() < 0.15);
    let mx = fvid_media::maxrgb_tonemap_pixel(255, 128, 64);
    assert!(mx.0 < 255);
    let merc =
        fvid_media::project_mercator_view(4, 2, &[0x00_20_20_20u32; 8], 2, 2, 0, 0, 90_000);
    assert_eq!(merc.len(), 4);
    assert_eq!(
        fvid_media::parse_spherical_projection("mercator").unwrap(),
        fvid_media::SphericalProjection::Mercator
    );
    let tb = fvid_media::dual_fisheye_tb_to_equirect(4, 4, &[0x00_11_11_11u32; 16], 4, 4);
    assert_eq!(tb.len(), 16);
    assert_eq!(fvid_media::clamp_live_latency_ms(50), 100);
    assert_eq!(
        fvid_media::buffer_health_ratio_milli(5_000_000, 10_000_000),
        500
    );
    assert!(fvid_media::format_epg_program_osd("News", 0, 3_600_000_000).contains("News"));
    assert!(fvid_media::format_cea708_service_osd(1).contains('1'));
    assert_eq!(
        fvid_media::parse_spherical_projection("octahedral").unwrap(),
        fvid_media::SphericalProjection::Octahedral
    );
    assert_eq!(
        fvid_media::parse_hdr_tonemap("maxrgb").unwrap(),
        fvid_media::HdrTonemap::MaxRgb
    );
    let oct = fvid_media::project_octahedral_view(
        4,
        4,
        &[0x00_30_30_30u32; 16],
        2,
        2,
        0,
        0,
        0,
        90_000,
    );
    assert_eq!(oct.len(), 4);
    let eq = fvid_media::project_equisolid_view(
        4,
        2,
        &[0x00_40_40_40u32; 8],
        2,
        2,
        0,
        0,
        0,
        120_000,
    );
    assert_eq!(eq.len(), 4);
    assert_eq!(fvid_media::clamp_paper_white_nits(203), 203);
    assert!(fvid_media::scale_sdr_overlay_to_paper_white(100, 203) > 100);
    assert!(fvid_media::format_st2094_l1_osd(1_000, 200).contains("L1"));
    assert_eq!(
        fvid_media::cycle_stereo_packing(fvid_media::StereoPacking::HalfSbs),
        fvid_media::StereoPacking::FullSbs
    );
    assert!(fvid_media::row_interleaved_eye_pixel(0, true));
    assert_eq!(fvid_media::passthrough_blend_milli(250), 750);
    assert_eq!(
        fvid_media::skip_segment_target_us(5_000_000, 0, 10_000_000),
        Some(10_000_000)
    );
    let maxrgb = fvid_media::apply_hdr_tonemap_pixel(
        200,
        100,
        50,
        fvid_media::HdrTonemap::MaxRgb,
        fvid_media::COLOR_TRC_SMPTE2084,
    );
    assert!(maxrgb.0 > 0);
    assert_eq!(
        fvid_media::parse_spherical_projection("orthographic").unwrap(),
        fvid_media::SphericalProjection::Orthographic
    );
    let ortho =
        fvid_media::project_orthographic_view(4, 2, &[0x00_55_55_55u32; 8], 2, 2, 0, 0, 0);
    assert_eq!(ortho.len(), 4);
    assert_eq!(fvid_media::clamp_diffuse_white_nits(203), 203);
    assert_eq!(fvid_media::map_nits_via_paper_white(203, 203, 1_000), 1_000);
    assert!(fvid_media::checkerboard_eye_is_left(0, 0));
    assert!(fvid_media::column_interleaved_eye_is_left(0));
    assert_eq!(fvid_media::wiggle_yaw_offset_milli(0, 5_000), 0);
    assert!(fvid_media::format_guardian_osd(100).contains("100"));
    assert_eq!(
        fvid_media::parse_spherical_projection("gnomonic").unwrap(),
        fvid_media::SphericalProjection::Gnomonic
    );
    assert_eq!(
        fvid_media::parse_spherical_projection("sinusoidal").unwrap(),
        fvid_media::SphericalProjection::Sinusoidal
    );
    let sin =
        fvid_media::project_sinusoidal_view(4, 2, &[0x00_66_66_66u32; 8], 2, 2, 0, 0, 90_000);
    assert_eq!(sin.len(), 4);
    assert_eq!(
        fvid_media::cubemap_cross_face_rect(400, 300, 0),
        (150, 75, 75, 75)
    );
    assert!(fvid_media::hlg_system_gamma_default_milli(1_000) >= 1_000);
    let mapped = fvid_media::apply_bt2020_to_bt709_pixel(200, 40, 40);
    assert_ne!(mapped, (200, 40, 40));
    assert_eq!(
        fvid_media::white_point_xy_milli(fvid_media::DisplayWhitePoint::D65),
        (3_127, 3_290)
    );
    assert_eq!(
        fvid_media::parse_edid_max_luminance("MaxLuminance=600"),
        Some(600)
    );
    assert_eq!(fvid_media::sample_1d_lut_u8(&[0, 128, 255], 128), 128);
    let mut gamut_opts = fvid_media::PlayRenderOptions::default();
    gamut_opts.gamut_map_bt709 = true;
    gamut_opts.color_primaries = fvid_media::COLOR_PRIMARIES_BT2020;
    let gpix = fvid_media::render_play_pixels(1, 1, &[0x00_c8_28_28u32], &gamut_opts, None);
    assert_ne!(gpix.2[0], 0x00_c8_28_28);
    assert_eq!(
        fvid_media::parse_spherical_projection("miller").unwrap(),
        fvid_media::SphericalProjection::Miller
    );
    let mill =
        fvid_media::project_miller_view(4, 2, &[0x00_77_77_77u32; 8], 2, 2, 0, 0, 90_000);
    assert_eq!(mill.len(), 4);
    let aeq = fvid_media::project_azimuthal_equidistant_view(
        4,
        2,
        &[0x00_88_88_88u32; 8],
        2,
        2,
        0,
        0,
        0,
    );
    assert_eq!(aeq.len(), 4);
    assert_eq!(fvid_media::hdr_brightness_boost_milli(203, 100), 406);
    assert_eq!(
        fvid_media::cycle_hdr_light_model(fvid_media::HdrLightModel::Display),
        fvid_media::HdrLightModel::Scene
    );
    assert_eq!(
        fvid_media::parse_cube_lut_1d_size("LUT_1D_SIZE 256"),
        Some(256)
    );
    assert_eq!(fvid_media::clamp_exclusive_latency_ms(0), 1);
    let mut hdr_opts = fvid_media::PlayRenderOptions::default();
    hdr_opts.hdr_tonemap = fvid_media::HdrTonemap::Hable;
    hdr_opts.color_trc = fvid_media::COLOR_TRC_SMPTE2084;
    hdr_opts.hdr_nits = 400;
    let gray = vec![0x00_80_80_80u32; 4];
    let base = fvid_media::render_play_pixels(
        2,
        2,
        &gray,
        &fvid_media::PlayRenderOptions {
            hdr_tonemap: fvid_media::HdrTonemap::Hable,
            color_trc: fvid_media::COLOR_TRC_SMPTE2084,
            hdr_nits: 100,
            ..fvid_media::PlayRenderOptions::default()
        },
        None,
    );
    let bright = fvid_media::render_play_pixels(2, 2, &gray, &hdr_opts, None);
    assert_ne!(base.2[0], bright.2[0]);
}

/// The curves behind `fvid media play --hdr-tonemap`, held to numbers instead
/// of to their own existence. Every mode is non-decreasing over 2000 steps of
/// the range a decoded channel actually arrives in, keeps black at black and
/// never leaves the displayable span. Reinhard, Hable, Möbius and ACES are then
/// pinned to sixteen points measured off this code. The ceiling alone would
/// not qualify them: both Hable and Möbius reach exactly 1.0 at 11.2, the white
/// the file normalises by, and both still report 1.0 there after the white is
/// pulled to 10 — only their 0.91803 and 0.98180 at 8.0 say which ceiling the
/// curve actually has. `pq_eotf` is ST 2084 written again from the standard's own
/// rationals and agrees to better than 1e-3 on a scale where full code is 100
/// — that is 100 cd/m² per unit, not the code's relative light. `hlg_eotf` is
/// twelve times BT.2100 scene light at every point probed (0.250000 against
/// 0.020833, 1.000000 against 0.083333, 3.179551 against 0.264963, 12.000002
/// against 1.000000): both of its branches lost the dividing 12 and this
/// records the factor as a named expectation, so repairing it has to change a
/// test rather than pass one silently. Möbius is checked against the native
/// track's own shoulder at the same joint and ceiling — after that curve was
/// rebuilt the two agree to six decimals across the range, which is more than
/// can be said for the modes whose numbers still do not transfer.
#[test]
fn the_legacy_hdr_curves_are_held_to_numbers() {
    let st2084 = |v: f64| {
        let m1 = 2610.0 / 16384.0;
        let m2 = 2523.0 / 4096.0 * 128.0;
        let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0);
        let vm = v.powf(1.0 / m2);
        ((vm - c1).max(0.0) / (c2 - c3 * vm).max(1e-9)).powf(1.0 / m1)
    };
    let bt2100_hlg = |e: f64| {
        let (a, b, c) = (0.17883277, 0.28466892, 0.55991073);
        if e <= 0.5 {
            (e * e) / 3.0
        } else {
            (((e - c) / a).exp() + b) / 12.0
        }
    };
    let modes = [
        fvid_media::HdrTonemap::Off,
        fvid_media::HdrTonemap::Clip,
        fvid_media::HdrTonemap::Reinhard,
        fvid_media::HdrTonemap::Hable,
        fvid_media::HdrTonemap::Mobius,
        fvid_media::HdrTonemap::Aces,
        fvid_media::HdrTonemap::MaxRgb,
    ];
    for mode in modes {
        let label = fvid_media::hdr_tonemap_label(mode);
        let mut previous = -1.0f32;
        for step in 0..=2000u16 {
            let x = 12.0 * f32::from(step) / 2000.0;
            let y = fvid_media::tonemap_channel(x, mode);
            assert!(
                y <= 1.0 + 1e-6,
                "{label} left the displayable span at {x}: {y}"
            );
            assert!(
                y + 1e-6 >= previous,
                "{label} went backwards from {previous} to {y} at {x}"
            );
            previous = y;
        }
        assert!(
            fvid_media::tonemap_channel(0.0, mode).abs() < 1e-6,
            "{label} lifted black"
        );
    }
    // Points measured off this code, to five decimals. A curve that clamps
    // cannot be qualified by its ceiling alone: normalise Hable by a smaller
    // white, or give Möbius a lower peak, and both still report exactly 1.0 at
    // 11.2 while saturating a stop early. The row below 11.2 is what names it.
    let anchors = [
        (fvid_media::HdrTonemap::Reinhard, 1.0f32, 0.5f32),
        (fvid_media::HdrTonemap::Reinhard, 5.0, 0.833_33),
        (fvid_media::HdrTonemap::Hable, 0.5, 0.171_97),
        (fvid_media::HdrTonemap::Hable, 1.0, 0.304_30),
        (fvid_media::HdrTonemap::Hable, 2.0, 0.492_92),
        (fvid_media::HdrTonemap::Hable, 5.0, 0.783_15),
        (fvid_media::HdrTonemap::Hable, 8.0, 0.918_03),
        (fvid_media::HdrTonemap::Hable, 11.2, 1.0),
        (fvid_media::HdrTonemap::Mobius, 0.5, 0.457_81),
        (fvid_media::HdrTonemap::Mobius, 1.0, 0.661_61),
        (fvid_media::HdrTonemap::Mobius, 2.0, 0.819_46),
        (fvid_media::HdrTonemap::Mobius, 5.0, 0.945_33),
        (fvid_media::HdrTonemap::Mobius, 8.0, 0.981_80),
        (fvid_media::HdrTonemap::Mobius, 11.2, 1.0),
        (fvid_media::HdrTonemap::Aces, 1.0, 0.803_80),
        (fvid_media::HdrTonemap::Aces, 2.0, 0.914_86),
    ];
    for (mode, x, expected) in anchors {
        let measured = fvid_media::tonemap_channel(x, mode);
        assert!(
            (measured - expected).abs() < 1e-4,
            "{} is {measured} at {x}, measured as {expected}",
            fvid_media::hdr_tonemap_label(mode)
        );
    }
    // Möbius stays 1:1 below its joint, then gives ground without inverting.
    for x in [0.0f32, 0.1, 0.25, 0.3] {
        assert_eq!(fvid_media::tonemap_channel(x, fvid_media::HdrTonemap::Mobius), x);
    }
    for i in 0..=48u16 {
        let x = 12.0 * f32::from(i) / 48.0;
        let legacy = fvid_media::tonemap_channel(x, fvid_media::HdrTonemap::Mobius);
        let native = fvid::color::curve(fvid::color::ToneMap::Mobius, x, 0.3, 11.2);
        assert!(
            (legacy - native).abs() < 1e-6,
            "the two tracks' Möbius disagree at {x}: legacy {legacy}, native {native}"
        );
    }
    for code in [0.25f32, 0.5, 0.75, 1.0] {
        let expected = st2084(f64::from(code)) * 100.0;
        let measured = f64::from(fvid_media::pq_eotf(code));
        assert!(
            (measured - expected).abs() < 1e-3,
            "pq_eotf({code}) is {measured}, ST 2084 on this scale is {expected}"
        );
        let expected = bt2100_hlg(f64::from(code)) * 12.0;
        let measured = f64::from(fvid_media::hlg_eotf(code));
        assert!(
            (measured - expected).abs() < 1e-4,
            "hlg_eotf({code}) is {measured}, twelve times BT.2100 scene light is {expected}"
        );
    }
    // The three transfer branches hand the same curve ranges that differ by
    // factors of the standard's own units; pinning them keeps a future
    // "fix" to one branch from quietly changing what the others mean.
    assert!(
        (fvid_media::expand_hdr_channel(1.0, fvid_media::COLOR_TRC_SMPTE2084) - 100.0).abs()
            < 1e-4
    );
    assert!((fvid_media::expand_hdr_channel(1.0, fvid_media::COLOR_TRC_HLG) - 12.0).abs() < 1e-4);
    assert_eq!(fvid_media::expand_hdr_channel(1.0, 1), 2.5);
}
