use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-sample-concat-metadata-preservation")
        .join(name)
}

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fvid"))
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-audio-sample-concat-meta-preserve-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sample_domain_audio_concat_preserves_and_edits_first_segment_metadata_across_wave_and_matroska_outputs() {
    let tmp = TempDir::new("concat-preserve");
    let wave_seg1 = fixture("wave-seg1-16bit.wav");
    let wave_seg2 = fixture("wave-seg2-24bit.wav");
    let mp4_seg1 = fixture("mp4-seg1.m4a");
    let mka_seg1 = fixture("matroska-seg1.mka");

    // 1. Mixed-depth WAVE (16-bit + 24-bit) sample-domain concat to `.wav` and `.mka`:
    //    preserves first-segment WAVE INFO tags (including ILNG -> stream language on .mka and QWER 4-CC)
    //    and selectively edits/deletes them without dropping untouched tags.
    let wave_pair = vec![wave_seg1.clone(), wave_seg2.clone()];
    let out_wave_wav_unmodified = tmp.0.join("wave-pair-unmodified.wav");
    fvid::media::concat(&wave_pair, &out_wave_wav_unmodified, &CopyOptions::default())
        .expect("concat 16-bit + 24-bit WAVE to .wav");
    let info_wave_wav = fvid::media::probe(&out_wave_wav_unmodified).unwrap();
    assert_eq!(info_wave_wav.streams[0].codec, "pcm_f32le");
    assert_eq!(
        info_wave_wav.metadata.get("title").map(String::as_str),
        Some("Concat Wave Seg 1 Title")
    );
    assert_eq!(
        info_wave_wav.metadata.get("artist").map(String::as_str),
        Some("Concat Wave Seg 1 Artist")
    );
    assert_eq!(
        info_wave_wav.metadata.get("album").map(String::as_str),
        Some("Concat Wave Seg 1 Album")
    );
    assert_eq!(
        info_wave_wav.metadata.get("comment").map(String::as_str),
        Some("Concat Wave Seg 1 Comment")
    );
    assert_eq!(
        info_wave_wav.metadata.get("copyright").map(String::as_str),
        Some("Concat Wave Seg 1 Copyright")
    );
    assert_eq!(
        info_wave_wav.metadata.get("encoder").map(String::as_str),
        Some("FVidConcatWaveSynth")
    );
    assert_eq!(
        info_wave_wav.metadata.get("track").map(String::as_str),
        Some("1/4")
    );
    assert_eq!(
        info_wave_wav.metadata.get("ILNG").map(String::as_str),
        Some("jpn")
    );
    assert_eq!(
        info_wave_wav.metadata.get("QWER").map(String::as_str),
        Some("ConcatWaveFourCC")
    );

    let out_wave_wav_edited = tmp.0.join("wave-pair-edited.wav");
    let edit_wave_wav_opts = CopyOptions {
        metadata_delete: vec!["comment".into(), "QWER".into()],
        metadata_set: vec![("title".into(), "Edited Wave Concat Title".into())],
        ..Default::default()
    };
    fvid::media::concat(&wave_pair, &out_wave_wav_edited, &edit_wave_wav_opts)
        .expect("concat 16-bit + 24-bit WAVE to .wav with selective edits");
    let info_wave_wav_ed = fvid::media::probe(&out_wave_wav_edited).unwrap();
    assert_eq!(
        info_wave_wav_ed.metadata.get("title").map(String::as_str),
        Some("Edited Wave Concat Title")
    );
    assert_eq!(
        info_wave_wav_ed.metadata.get("artist").map(String::as_str),
        Some("Concat Wave Seg 1 Artist")
    );
    assert_eq!(
        info_wave_wav_ed.metadata.get("album").map(String::as_str),
        Some("Concat Wave Seg 1 Album")
    );
    assert!(!info_wave_wav_ed.metadata.contains_key("comment"));
    assert!(!info_wave_wav_ed.metadata.contains_key("QWER"));

    let out_wave_mka_unmodified = tmp.0.join("wave-pair-unmodified.mka");
    fvid::media::concat(&wave_pair, &out_wave_mka_unmodified, &CopyOptions::default())
        .expect("concat 16-bit + 24-bit WAVE to .mka");
    let info_wave_mka = fvid::media::probe(&out_wave_mka_unmodified).unwrap();
    assert_eq!(
        info_wave_mka.metadata.get("title").map(String::as_str),
        Some("Concat Wave Seg 1 Title")
    );
    assert_eq!(
        info_wave_mka.metadata.get("artist").map(String::as_str),
        Some("Concat Wave Seg 1 Artist")
    );
    assert_eq!(
        info_wave_mka.metadata.get("album").map(String::as_str),
        Some("Concat Wave Seg 1 Album")
    );
    assert_eq!(
        info_wave_mka.metadata.get("ENCODER").map(String::as_str),
        Some("FVidConcatWaveSynth")
    );
    assert_eq!(
        info_wave_mka.metadata.get("QWER").map(String::as_str),
        Some("ConcatWaveFourCC")
    );
    assert_eq!(
        info_wave_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("jpn")
    );

    // 2. MP4 first segment + WAVE second segment sample-domain concat to `.wav` and `.mka`:
    //    preserves MP4 container and track 0 metadata and selectively edits/deletes them.
    let mp4_pair = vec![mp4_seg1.clone(), wave_seg2.clone()];
    let out_mp4_wav = tmp.0.join("mp4-pair.wav");
    let edit_mp4_wav_opts = CopyOptions {
        metadata_delete: vec!["comment".into()],
        metadata_set: vec![("title".into(), "Edited MP4 Wave Concat Title".into())],
        ..Default::default()
    };
    fvid::media::concat(&mp4_pair, &out_mp4_wav, &edit_mp4_wav_opts)
        .expect("concat MP4 + WAVE to .wav");
    let info_mp4_wav = fvid::media::probe(&out_mp4_wav).unwrap();
    assert_eq!(
        info_mp4_wav.metadata.get("title").map(String::as_str),
        Some("Edited MP4 Wave Concat Title")
    );
    assert_eq!(
        info_mp4_wav.metadata.get("artist").map(String::as_str),
        Some("Concat MP4 Artist")
    );
    assert_eq!(
        info_mp4_wav.metadata.get("album").map(String::as_str),
        Some("Concat MP4 Album")
    );
    assert_eq!(
        info_mp4_wav.metadata.get("copyright").map(String::as_str),
        Some("Concat MP4 Copyright")
    );
    assert_eq!(
        info_mp4_wav.metadata.get("track").map(String::as_str),
        Some("3/7")
    );
    assert_eq!(
        info_mp4_wav.metadata.get("ILNG").map(String::as_str),
        Some("fra")
    );
    assert!(!info_mp4_wav.metadata.contains_key("comment"));

    let out_mp4_mka = tmp.0.join("mp4-pair.mka");
    let edit_mp4_mka_opts = CopyOptions {
        metadata_delete: vec!["comment".into(), "description".into()],
        metadata_set: vec![
            ("title".into(), "Edited MP4 MKA Concat Title".into()),
            ("publisher".into(), "MP4 Concat Publisher".into()),
        ],
        stream_metadata_set: vec![(0, "ISRC".into(), "FR-FVD-26-00010".into())],
        ..Default::default()
    };
    fvid::media::concat(&mp4_pair, &out_mp4_mka, &edit_mp4_mka_opts)
        .expect("concat MP4 + WAVE to .mka");
    let info_mp4_mka = fvid::media::probe(&out_mp4_mka).unwrap();
    assert_eq!(
        info_mp4_mka.metadata.get("title").map(String::as_str),
        Some("Edited MP4 MKA Concat Title")
    );
    assert_eq!(
        info_mp4_mka.metadata.get("artist").map(String::as_str),
        Some("Concat MP4 Artist")
    );
    assert_eq!(
        info_mp4_mka.metadata.get("album_artist").map(String::as_str),
        Some("Concat MP4 Album Artist")
    );
    assert_eq!(
        info_mp4_mka.metadata.get("disc").map(String::as_str),
        Some("1/2")
    );
    assert_eq!(
        info_mp4_mka.metadata.get("publisher").map(String::as_str),
        Some("MP4 Concat Publisher")
    );
    assert!(!info_mp4_mka.metadata.contains_key("comment"));
    assert!(!info_mp4_mka.metadata.contains_key("description"));
    assert_eq!(
        info_mp4_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Concat MP4 Track Title")
    );
    assert_eq!(
        info_mp4_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert_eq!(
        info_mp4_mka.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("FR-FVD-26-00010")
    );

    // 3. Matroska first segment + WAVE second segment sample-domain concat to `.wav` and `.mka` in API and CLI:
    //    preserves Matroska container/track tags (including WritingApp encoder, custom container tags,
    //    and custom track-scoped tags) and selectively edits/deletes them.
    let mka_pair = vec![mka_seg1.clone(), wave_seg2.clone()];
    let out_mka_wav = tmp.0.join("mka-pair.wav");
    fvid::media::concat(&mka_pair, &out_mka_wav, &CopyOptions::default())
        .expect("concat MKA + WAVE to .wav");
    let info_mka_wav = fvid::media::probe(&out_mka_wav).unwrap();
    assert_eq!(
        info_mka_wav.metadata.get("title").map(String::as_str),
        Some("Concat MKA Title")
    );
    assert_eq!(
        info_mka_wav.metadata.get("artist").map(String::as_str),
        Some("Concat MKA Artist")
    );
    assert_eq!(
        info_mka_wav.metadata.get("encoder").map(String::as_str),
        Some("FVidConcatMkaSynth")
    );
    assert_eq!(
        info_mka_wav.metadata.get("ILNG").map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        info_mka_wav.metadata.get("QWER").map(String::as_str),
        Some("ConcatMkaFourCC")
    );

    let cli_mka_out = tmp.0.join("cli-mka-pair.mka");
    let cli_res = Command::new(bin_path())
        .args([
            "media",
            "concat",
            cli_mka_out.to_str().unwrap(),
            mka_seg1.to_str().unwrap(),
            wave_seg2.to_str().unwrap(),
            "--metadata-delete",
            "comment",
            "--metadata-delete",
            "custom_concat_source",
            "--metadata",
            "title=CLI Concat MKA Edited Title",
            "--stream-metadata-delete",
            "0:custom_concat_track",
            "--stream-metadata",
            "0:language=spa",
            "-q",
        ])
        .output()
        .expect("cli concat MKA + WAVE to .mka");
    assert!(
        cli_res.status.success(),
        "cli concat failed: {}",
        String::from_utf8_lossy(&cli_res.stderr)
    );
    let info_cli_mka = fvid::media::probe(&cli_mka_out).unwrap();
    assert_eq!(
        info_cli_mka.metadata.get("title").map(String::as_str),
        Some("CLI Concat MKA Edited Title")
    );
    assert_eq!(
        info_cli_mka.metadata.get("artist").map(String::as_str),
        Some("Concat MKA Artist")
    );
    assert_eq!(
        info_cli_mka.metadata.get("album").map(String::as_str),
        Some("Concat MKA Album")
    );
    assert_eq!(
        info_cli_mka.metadata.get("ENCODER").map(String::as_str),
        Some("FVidConcatMkaSynth")
    );
    assert!(!info_cli_mka.metadata.contains_key("comment"));
    assert!(!info_cli_mka
        .metadata
        .contains_key("CUSTOM_CONCAT_SOURCE"));
    assert_eq!(
        info_cli_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Concat MKA Track Title")
    );
    assert_eq!(
        info_cli_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("spa")
    );
    assert_eq!(
        info_cli_mka.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00003")
    );
    assert!(!info_cli_mka.streams[0]
        .metadata
        .contains_key("CUSTOM_CONCAT_TRACK"));

    // 4. Tight controlled memory budget rejects sample-domain WAVE concat and plan_concat.
    let tight_mem = CopyOptions {
        max_controlled_bytes: Some(16),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&wave_pair, &tight_mem).is_err());
    assert!(fvid::media::concat(&wave_pair, &tmp.0.join("never-mem.wav"), &tight_mem).is_err());
    assert!(fvid::media::concat(&wave_pair, &tmp.0.join("never-mem.mka"), &tight_mem).is_err());
    assert!(fvid::media::plan_decode_audio(&wave_seg1, &Default::default(), &tight_mem).is_err());
}
