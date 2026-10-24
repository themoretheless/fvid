#![cfg(feature = "player")]

use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fvid::playback_native::NativeReader;
use fvid::playback_thread::{Event, Playback};
use fvid_media::owned_play_controls::{
    apply_box_denoise_pixel, apply_unsharp_pixel, blit_bitmap_subtitle, detect_letterbox_bars,
    fit_window_to_video, locked_window_size, parse_subtitle_text, waveform_column_fills,
    BitmapSubtitle,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("play-controls")
}

#[test]
fn playback_thread_seek_error_halts_worker_instead_of_resuming_decode() {
    let bytes = std::fs::read(fixture_dir().join("two-frame-420.y4m")).unwrap();
    let reader = NativeReader::software(Cursor::new(bytes), 1024 * 1024).unwrap();
    let mut playback = Playback::start(reader, None);

    // Drain initial frame (generation 0).
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got_initial = false;
    while Instant::now() < deadline {
        if let Some(Event::Frame(f)) = playback.poll() {
            assert_eq!(f.generation, 0);
            got_initial = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(got_initial, "must receive initial frame");

    // Pause before seeking so the worker does not decode frame 2 before the seek command.
    playback.pause();
    while playback.poll().is_some() {}

    // Resume and seek on an unseekable Y4M stream (generation 1): seek_raw returns Err.
    playback.play();
    playback.seek(Duration::from_secs(10));

    let mut errors = Vec::new();
    let mut post_error_frames = 0usize;
    let mut post_error_ended = 0usize;
    let wait_until = Instant::now() + Duration::from_millis(120);
    while Instant::now() < wait_until {
        if let Some(event) = playback.poll() {
            match event {
                Event::Error(e) => errors.push(e),
                Event::Frame(f) if f.generation == 1 => post_error_frames += 1,
                Event::Ended(1) => post_error_ended += 1,
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(errors.len(), 1, "expected exactly 1 seek error event, got {errors:?}");
    assert_eq!(
        (post_error_frames, post_error_ended),
        (0, 0),
        "worker must set ended=true after a seek error and not continue decoding generation 1 (frames={post_error_frames}, ended={post_error_ended})"
    );
}

#[test]
fn play_controls_ass_with_arrow_in_dialogue_and_scoped_events() {
    let text = std::fs::read_to_string(fixture_dir().join("arrow-in-dialogue.ass")).unwrap();
    let cues = parse_subtitle_text(&text)
        .expect("ASS document containing '-->' in comments/dialogue must parse as ASS");
    assert_eq!(
        cues.len(),
        1,
        "only Dialogue lines inside [Events] should be parsed, got {cues:?}"
    );
    assert_eq!(cues[0].start_us, 1_000_000);
    assert_eq!(cues[0].end_us, 3_000_000);
    assert_eq!(cues[0].text, "Step A --> Step B");
}

#[test]
fn play_controls_blit_bitmap_subtitle_extreme_offsets_do_not_overflow() {
    let mut frame = vec![0x00112233u32; 4];
    let sub_pos = BitmapSubtitle {
        start_us: 0,
        end_us: 1_000_000,
        x: i32::MAX,
        y: i32::MAX,
        width: 2,
        height: 2,
        pixels: vec![0xFF_FF_00_00; 4],
    };
    blit_bitmap_subtitle(&mut frame, 2, 2, &sub_pos);
    assert_eq!(frame, vec![0x00112233u32; 4]);

    let sub_neg = BitmapSubtitle {
        start_us: 0,
        end_us: 1_000_000,
        x: i32::MIN,
        y: i32::MIN,
        width: 2,
        height: 2,
        pixels: vec![0xFF_FF_00_00; 4],
    };
    blit_bitmap_subtitle(&mut frame, 2, 2, &sub_neg);
    assert_eq!(frame, vec![0x00112233u32; 4]);
}

#[test]
fn play_controls_pixel_helpers_large_dimensions_and_fit_window_bounds() {
    // 65536 * 65536 = 2^32, which wraps to 0 in u32 multiplication if not widened first!
    let pixels = [0x00FF_FFFFu32; 4];
    assert_eq!(detect_letterbox_bars(65_536, 65_536, &pixels, 16), (0, 0, 0, 0));
    assert_eq!(waveform_column_fills(65_536, 65_536, &pixels, 4), vec![0, 0, 0, 0]);
    assert_eq!(apply_box_denoise_pixel(65_536, 65_536, &pixels, 0, 0, 500), pixels[0]);
    assert_eq!(apply_unsharp_pixel(65_536, 65_536, &pixels, 0, 0, 500), pixels[0]);

    let (fw, fh) = fit_window_to_video(16_777_217, 16_777_217, 1_920, 1_080);
    assert!(fw <= 1_920 && fh <= 1_080, "fitted window ({fw}x{fh}) exceeded max bounds");
    let (lw, lh) = locked_window_size(16_777_217, 16_777_217, 1_920, 1_080, true);
    assert!(lw <= 1_920 && lh <= 1_080, "locked window ({lw}x{lh}) exceeded max bounds");
}
