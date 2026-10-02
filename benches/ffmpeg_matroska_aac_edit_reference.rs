//! Explicit external AAC edit-boundary reference comparison.
use fvid::container::matroska_write;
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-mka-mux-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
fn independent_decoder_reads_mp4_aac_edit_boundaries() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir("mp4-edit-reference");
    let bytes = std::fs::read(fixture("aac-native-edit.m4a")).unwrap();
    let mut input =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    let index = input
        .tracks()
        .iter()
        .position(|t| t.codec == *b"mp4a")
        .unwrap();
    let mut out = std::io::Cursor::new(Vec::new());
    matroska_write::write_mp4_aac(&mut input, index, &mut out, None, None).unwrap();
    let destination = d.0.join("edited.mka");
    std::fs::write(&destination, out.into_inner()).unwrap();
    let decode = |path: &Path| {
        let run = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-f",
                "f32le",
                "-c:a",
                "pcm_f32le",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        run.stdout
    };
    let source_pcm = decode(&fixture("aac-native-edit.m4a"));
    let output_pcm = decode(&destination);
    let mut owned = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut owned).unwrap();
    let length = stats.sample_frames as usize * 4;
    assert_eq!(output_pcm.len(), length);
    assert!(source_pcm.len() >= length);
    assert!(output_pcm == source_pcm[..length], "reference PCM differs");
}


fn main() {
    std::env::var("FVID_REFERENCE_FFMPEG").expect("set FVID_REFERENCE_FFMPEG");
    independent_decoder_reads_mp4_aac_edit_boundaries();
    println!("Matroska AAC edit-boundary reference suite passed");
}
