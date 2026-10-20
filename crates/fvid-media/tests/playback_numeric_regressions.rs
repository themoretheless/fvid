use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/player-controls")
}
fn inputs() -> serde_json::Value {
    assert_eq!(
        fvid_media::owned_y4m_decode::decode_video(&root().join("control.y4m"))
            .unwrap()
            .video_frames,
        3
    );
    serde_json::from_slice(&fs::read(root().join("numeric-controls.json")).unwrap()).unwrap()
}

#[test]
fn minimum_signed_offsets_format_without_overflow() {
    let delta = inputs()["display_delta_us"].as_i64().unwrap();
    assert_eq!(
        fvid_media::format_drag_seek_osd(delta),
        "Drag -2562047788:00:54"
    );
    assert_eq!(
        fvid_media::format_watch_party_osd(true, delta),
        "Watch party sync -2562047788:00:54"
    );
    assert_eq!(fvid_media::format_drag_seek_osd(-1_000_000), "Drag -00:01");
    assert_eq!(
        fvid_media::format_watch_party_osd(false, delta),
        "Watch party Off"
    );
}

#[test]
fn clock_sync_midpoint_cannot_overflow_the_difference() {
    let input = inputs();
    let local = input["sync_local_us"].as_i64().unwrap();
    let peer = input["sync_peer_us"].as_i64().unwrap();
    assert_eq!(fvid_media::multi_room_sync_target_us(local, peer, 0), 0);
    assert_eq!(fvid_media::multi_room_sync_target_us(peer, local, 0), -1);
    assert_eq!(
        fvid_media::multi_room_sync_target_us(local, local, 0),
        local
    );
    assert_eq!(fvid_media::multi_room_sync_target_us(peer, peer, 0), peer);
    assert_eq!(fvid_media::multi_room_sync_target_us(10, 9, 0), 10);
    assert_eq!(fvid_media::multi_room_sync_target_us(100, 200, 20), 160);
}

#[test]
fn proportional_seeks_preserve_the_ratio_before_narrowing() {
    let duration = inputs()["duration_us"].as_i64().unwrap();
    assert_eq!(
        fvid_media::position_us_from_digit(0, duration),
        Some(duration)
    );
    assert_eq!(
        fvid_media::position_us_from_digit(9, duration),
        Some(900_000_000_000_000_000)
    );
    assert_eq!(
        fvid_media::thumbnail_seek_us(19, 20, duration),
        950_000_000_000_000_000
    );
    assert_eq!(
        fvid_media::thumbnail_seek_us(usize::MAX, 20, duration),
        950_000_000_000_000_000
    );
    assert_eq!(fvid_media::thumbnail_seek_us(0, usize::MAX, duration), 0);
    assert_eq!(fvid_media::thumbnail_seek_us(1, 0, duration), 0);
    assert_eq!(
        fvid_media::position_us_from_digit(1, 1_000_000),
        Some(100_000)
    );
}

#[test]
fn large_control_steps_clamp_or_wrap_without_panicking() {
    type Step = fn(i32, i32) -> i32;
    let steps: &[(Step, i32, i32)] = &[
        (fvid_media::tone_gain_step_milli, 0, 2000),
        (fvid_media::adjust_step_milli, 0, 2000),
        (fvid_media::balance_step_milli, 0, 2000),
        (fvid_media::width_step_milli, 0, 2000),
        (fvid_media::crossfeed_step_milli, 0, 1000),
        (fvid_media::subtitle_scale_step_milli, 500, 2000),
        (fvid_media::subtitle_opacity_step_milli, 100, 1000),
        (fvid_media::eq_band_step_milli, 50, 4000),
        (fvid_media::pitch_step_milli, -89_000, 89_000),
        (fvid_media::fov_step_milli, 20_000, 150_000),
    ];
    for &(step, low, high) in steps {
        assert_eq!(step(1000, i32::MAX), high);
        assert_eq!(step(-1000, i32::MIN), low);
    }
    assert_eq!(fvid_media::volume_step_milli(1000, i32::MAX), 2000);
    assert_eq!(fvid_media::volume_step_milli(u32::MAX, 0), 2000);
    assert_eq!(fvid_media::rate_step_milli(1000, i32::MAX), 4000);
    assert_eq!(fvid_media::rate_step_milli(0, i32::MIN), 250);
    assert_eq!(fvid_media::yaw_step_milli(350_000, 10_000), 0);
    assert_eq!(fvid_media::roll_step_milli(350_000, 10_000), 0);
    // Exact addition modulo a full revolution, with no signed 32-bit narrowing.
    assert_eq!(fvid_media::yaw_step_milli(i32::MAX, i32::MAX), 167_294);
    assert_eq!(fvid_media::roll_step_milli(i32::MAX, i32::MAX), 167_294);
    assert_eq!(fvid_media::pan_step_px(1000, i32::MAX, 320, 640), 160);
    assert_eq!(fvid_media::pan_step_px(-1000, i32::MIN, 320, 640), -160);
}

#[test]
fn audio_skew_drops_only_available_frames_at_signed_boundaries() {
    inputs();
    assert_eq!(
        fvid_media::step_audio_skew(i64::MIN, 2, 2),
        (i64::MIN + 2, false, 2)
    );
    assert_eq!(fvid_media::step_audio_skew(-4, 2, u64::MAX), (-2, false, 2));
    assert_eq!(fvid_media::step_audio_skew(-4, 5, 2), (-2, false, 2));
    assert_eq!(fvid_media::step_audio_skew(4, 2, 0), (2, true, 0));
    assert_eq!(
        fvid_media::step_audio_skew(i64::MIN, 2, 0),
        (i64::MIN, false, 0)
    );
}

#[test]
fn track_and_output_device_cycles_do_not_overflow_before_modulo() {
    inputs();
    assert_eq!(fvid_media::cycle_track(3, 2, i32::MAX, false), 0);
    assert_eq!(fvid_media::cycle_track(3, 2, i32::MAX, true), 1);
    assert_eq!(fvid_media::cycle_track(3, 0, i32::MIN, false), 1);
    assert_eq!(fvid_media::cycle_track(0, 0, i32::MAX, false), -1);
    // Explicit refusal: ordinal i32 cannot describe every entry of this list.
    assert_eq!(fvid_media::cycle_track(usize::MAX, 0, 1, false), -1);
    assert_eq!(fvid_media::cycle_video_track(3, u32::MAX), 1);
    assert_eq!(
        fvid_media::format_video_track_osd(u32::MAX, 3),
        "Video 4294967296/3"
    );
    let names = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
    assert_eq!(
        fvid_media::cycle_output_device(&names, "c", i32::MAX),
        Some("a")
    );
    assert_eq!(
        fvid_media::cycle_output_device(&names, "a", i32::MIN),
        Some("b")
    );
}
