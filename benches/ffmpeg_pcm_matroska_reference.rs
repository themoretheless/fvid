use fvid::{
    container::{
        matroska_write::{Encoding, PacketWriter, TrackSpec},
        webm,
    },
    media_control::{CancelFlag, ProgressHook},
    native_export,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!("fvid-pcm-mka-{name}-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    Dir(path)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn export(source: &Path, output: &Path) -> fvid::native_media::AudioDecodeStats {
    native_export::export_audio_pcm_selected(
        source, output, None, 1.0, None, None, None, None, None,
    )
    .unwrap()
}
fn pcm_mux_round_trips_aac_edits_alac_and_multichannel_samples_exactly() {
    let d = dir("roundtrip");
    for (i, name) in [
        "audio/aac-stereo.aac",
        "audio/aac-native-edit.m4a",
        "alac/stereo-24.m4a",
        "alac/stereo-24.mka",
        "audio/aac-51-active.aac",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let reference = d.0.join(format!("raw-{i}.f32le"));
        let muxed = d.0.join(format!("pcm-{i}.mka"));
        let decoded = d.0.join(format!("decoded-{i}.f32le"));
        let original = export(&source, &reference);
        let written = export(&source, &muxed);
        let read = export(&muxed, &decoded);
        assert_eq!(original.sample_frames, written.sample_frames);
        assert_eq!(read.sample_frames, written.sample_frames);
        let raw = std::fs::read(reference).unwrap();
        let actual = std::fs::read(decoded).unwrap();
        assert!(raw == actual, "{name} samples differ");
        let bytes = std::fs::read(&muxed).unwrap();
        let mut reader = webm::WebmReader::open(Cursor::new(bytes), Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].codec, "A_PCM/FLOAT/IEEE");
        assert_eq!(reader.tracks[0].bit_depth, 32);
        assert_eq!(reader.tracks[0].channels, u64::from(written.channels));
        let mut frames = 0u128;
        for packet in &reader.packets {
            assert_eq!(
                packet.pts_ns as u128,
                frames * 1_000_000_000 / u128::from(written.sample_rate)
            );
            frames += (packet.size / (usize::from(written.channels) * 4)) as u128;
            assert!(packet.size <= 1024 * usize::from(written.channels) * 4);
            assert_eq!(
                packet.pts_ns as u128 + u128::from(packet.duration_ns.unwrap()),
                frames * 1_000_000_000 / u128::from(written.sample_rate)
            );
        }
        assert_eq!(frames, u128::from(written.sample_frames));
        if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let result = std::process::Command::new(ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&muxed)
                .args(["-f", "f32le", "-"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(result.stdout == raw, "independent PCM differs: {name}");
        }
    }
}

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    pcm_mux_round_trips_aac_edits_alac_and_multichannel_samples_exactly();
    println!("PCM Matroska AAC, ALAC and multichannel reference comparisons passed");
}
