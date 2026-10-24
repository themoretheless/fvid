use std::path::Path;
use std::process::Command;

#[test]
fn player_cli_rejects_oversized_lut_and_skin_files_cleanly() {
    let bin = env!("CARGO_BIN_EXE_fvid");
    let temp = std::env::temp_dir().join(format!("fvid-player-cli-bounds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let huge_skin = temp.join("huge-skin.json");
    std::fs::write(&huge_skin, vec![b' '; 64 * 1024 + 2]).unwrap();
    let out = Command::new(bin)
        .args(["play", "--skin", huge_skin.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skin file is too large"),
        "unexpected stderr for huge skin: {stderr}"
    );

    let huge_lut = temp.join("huge.cube");
    std::fs::write(&huge_lut, vec![b'#'; 16 * 1024 * 1024 + 2]).unwrap();
    let out = Command::new(bin)
        .args(["play", "--lut", huge_lut.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("LUT file is too large"),
        "unexpected stderr for huge lut: {stderr}"
    );

    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-queue");
    for name in ["queue.pls", "queue.m3u", "clip.avi", "clip.wv", "companion.wvc"] {
        assert!(fixture_dir.join(name).is_file(), "missing fixture {name}");
    }

    std::fs::remove_dir_all(&temp).unwrap();
}
