//! Explicit RGBA oracle, kept outside ordinary tests.
use std::process::Command;
fn main() {
    let oracle = std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference required");
    let names = include_str!("../crates/fvid-media/src/owned_color_names.rs")
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("(\"")
                .and_then(|s| s.split_once('"').map(|p| p.0))
        });
    let mut count = 0;
    for name in names.chain([
        "0xff0050@.4",
        "0x11223344@.5",
        "#11223344",
        "blue@0x40",
        "white@0",
    ]) {
        let color = fvid_media::owned_rgba::parse(name).unwrap();
        let output = Command::new(&oracle)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={name}:s=2x2,format=rgba"),
                "-threads",
                "1",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, color.repeat(4), "{name}");
        count += 1;
    }
    println!("RGBA: {count} exact named/hex/alpha color comparisons passed");
}
