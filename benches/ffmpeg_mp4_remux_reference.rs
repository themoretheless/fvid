//! Explicit MP4 to Matroska video and audio reference comparisons.
use fvid::container::{mp4, mp4_matroska};
use std::{io::Cursor, path::{Path, PathBuf}};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn remux(bytes: &[u8]) -> Vec<u8> {
    let mut input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    let stats = mp4_matroska::write(&mut input, &mut output, None, None).unwrap();
    assert!(!stats.done);
    assert!(stats.packets > 0);
    output.into_inner()
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory(label: &str) -> Directory {
    let p = std::env::temp_dir().join(format!("fvid-mp4-matroska-{label}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Directory(p)
}
fn independent_decoder_preserves_video_and_audio_for_combined_files() {
    use std::process::Command;
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let dir = directory("reference");
    for name in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "audio/two-audio.mp4",
    ] {
        let source = dir.0.join("source.mp4");
        if name == "audio/two-audio.mp4" {
            std::fs::copy(fixture(name), &source).unwrap();
        } else {
            let run = Command::new(&ffmpeg)
                .args(["-v", "error", "-y", "-i"])
                .arg(fixture(name))
                .arg("-i")
                .arg(fixture("audio/aac-native-edit.m4a"))
                .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
        let bytes = std::fs::read(&source).unwrap();
        let output = remux(&bytes);
        let dest = dir.0.join("result.mkv");
        std::fs::write(&dest, &output).unwrap();
        let decode = |path: &Path, map: &str, audio: bool| {
            let mut cmd = Command::new(&ffmpeg);
            cmd.args(["-v", "error", "-i"])
                .arg(path)
                .args(["-map", map]);
            if audio {
                cmd.args(["-f", "f32le", "-c:a", "pcm_f32le"]);
            } else {
                cmd.args([
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    if name.contains("main10") {
                        "yuv420p10le"
                    } else {
                        "yuv420p"
                    },
                ]);
            }
            let run = cmd.arg("pipe:1").output().unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            run.stdout
        };
        assert!(
            decode(&source, "0:v:0", false) == decode(&dest, "0:v:0", false),
            "{name}: video"
        );
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut audio_index = 0;
        for track in input.tracks().iter().filter(|t| t.handler == *b"soun") {
            let map = format!("0:a:{audio_index}");
            audio_index += 1;
            let before = decode(&source, &map, true);
            let after = decode(&dest, &map, true);
            let samples = if track.edits.is_empty() {
                u128::from(track.duration) * u128::from(track.sample_rate)
                    / u128::from(track.timescale)
            } else {
                (u128::from(track.edits[0].duration) * u128::from(track.sample_rate))
                    .div_ceil(u128::from(input.movie_timescale()))
            };
            let length = samples as usize * usize::from(track.channels) * 4;
            assert_eq!(after.len(), length, "{name} {map}: audible samples");
            assert!(before.len() >= length);
            assert!(after == before[..length], "{name} {map}: audio");
        }
    }
}


fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    independent_decoder_preserves_video_and_audio_for_combined_files();
    println!("MP4 Matroska video and audio remux reference comparisons passed");
}
