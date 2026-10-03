#![cfg(feature = "player")]
#[test]
fn media_play_device_listing_uses_native_player_without_media_feature() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "play", "--list-audio-devices"])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Hosts with no sound hardware may enumerate no devices; success still
    // proves this command reached the native pre-window enumeration path.
    assert!(!String::from_utf8_lossy(&result.stderr).contains("requires --features media"));
}
