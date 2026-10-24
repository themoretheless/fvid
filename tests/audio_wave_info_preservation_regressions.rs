use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-wave-info-preservation")
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
            "fvid-audio-wave-info-preservation-{tag}-{}-{}",
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
fn wave_mp4_and_matroska_audio_exports_preserve_and_edit_source_wave_info_tags_across_decode_trim_loudnorm_and_normalize() {
    let tmp = TempDir::new("preserve");
    let wav = fixture("wave-info-preserve.wav");
    let mp4 = fixture("mp4-info-preserve.m4a");
    let mka = fixture("matroska-info-preserve.mka");

    // 0. Verify source probes expose expected tags.
    let wav_probe = fvid::media::probe(&wav).expect("probe wave fixture");
    assert_eq!(
        wav_probe.metadata.get("title").map(String::as_str),
        Some("Wave Native Title")
    );
    assert_eq!(
        wav_probe.metadata.get("QWER").map(String::as_str),
        Some("OpaqueWaveTag")
    );

    let mp4_probe = fvid::media::probe(&mp4).expect("probe mp4 fixture");
    assert_eq!(
        mp4_probe.metadata.get("title").map(String::as_str),
        Some("MP4 Source Title")
    );
    assert_eq!(
        mp4_probe.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    let mka_probe = fvid::media::probe(&mka).expect("probe mka fixture");
    assert_eq!(
        mka_probe.metadata.get("title").map(String::as_str),
        Some("MKA Source Title")
    );
    assert_eq!(
        mka_probe.metadata.get("encoder").map(String::as_str),
        Some("FVidSynthMuxer")
    );
    assert_eq!(
        mka_probe.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00001")
    );

    // 1. Default `decode_audio` and `normalize_loudness_file_with_options` on WAVE (routed via `native_export::export_pcm_selected`)
    //    must preserve source RIFF INFO chunks including opaque 4-CC tags!
    let out_wav_decode = tmp.0.join("wav-decode-default.wav");
    fvid::media::decode_audio(&wav, &out_wav_decode, &CopyOptions::default())
        .expect("decode_audio wave default");
    let probed_wav_decode = fvid::media::probe(&out_wav_decode).unwrap();
    assert_eq!(
        probed_wav_decode.metadata.get("title").map(String::as_str),
        Some("Wave Native Title")
    );
    assert_eq!(
        probed_wav_decode.metadata.get("artist").map(String::as_str),
        Some("Wave Native Artist")
    );
    assert_eq!(
        probed_wav_decode.metadata.get("comment").map(String::as_str),
        Some("Wave Native Comment")
    );
    assert_eq!(
        probed_wav_decode
            .metadata
            .get("copyright")
            .map(String::as_str),
        Some("Wave Native Copyright")
    );
    assert_eq!(
        probed_wav_decode.metadata.get("ILNG").map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        probed_wav_decode.metadata.get("QWER").map(String::as_str),
        Some("OpaqueWaveTag")
    );

    let out_wav_norm = tmp.0.join("wav-norm-default.wav");
    let weights = fvid::native_pcm::loudness_channel_weights(&wav, None).unwrap();
    fvid::native_pcm::normalize_loudness_file_with_options(
        &wav,
        &out_wav_norm,
        &weights,
        fvid::native_pcm::NormalizeTarget::default(),
        &CopyOptions::default(),
        None,
    )
    .expect("normalize-loudness wave default");
    let probed_wav_norm = fvid::media::probe(&out_wav_norm).unwrap();
    assert_eq!(
        probed_wav_norm.metadata.get("title").map(String::as_str),
        Some("Wave Native Title")
    );
    assert_eq!(
        probed_wav_norm.metadata.get("QWER").map(String::as_str),
        Some("OpaqueWaveTag")
    );

    // 2. MP4 -> .wav across `decode_audio` (both default native route and `fvid_media` route), `trim`, `loudnorm`, and `normalize-loudness`
    //    must preserve source MP4 container & track tags in RIFF INFO and support selective override/delete.
    let out_mp4_decode = tmp.0.join("mp4-decode-default.wav");
    fvid::media::decode_audio(&mp4, &out_mp4_decode, &CopyOptions::default())
        .expect("decode_audio mp4 default");
    let probed_mp4_decode = fvid::media::probe(&out_mp4_decode).unwrap();
    assert_eq!(
        probed_mp4_decode.metadata.get("title").map(String::as_str),
        Some("MP4 Source Title")
    );
    assert_eq!(
        probed_mp4_decode.metadata.get("artist").map(String::as_str),
        Some("MP4 Source Artist")
    );
    assert_eq!(
        probed_mp4_decode.metadata.get("album").map(String::as_str),
        Some("MP4 Source Album")
    );
    assert_eq!(
        probed_mp4_decode.metadata.get("date").map(String::as_str),
        Some("2026")
    );
    assert_eq!(
        probed_mp4_decode
            .metadata
            .get("comment")
            .map(String::as_str),
        Some("MP4 Source Comment")
    );
    assert_eq!(
        probed_mp4_decode
            .metadata
            .get("copyright")
            .map(String::as_str),
        Some("MP4 Source Copyright")
    );
    assert_eq!(
        probed_mp4_decode.metadata.get("track").map(String::as_str),
        Some("5/12")
    );
    assert_eq!(
        probed_mp4_decode.metadata.get("ILNG").map(String::as_str),
        Some("fra")
    );

    // Selective edit on MP4 -> .wav via `decode_audio_transformed` (routes through `fvid_media::owned_mp4_audio_export`)
    let mp4_edit_opts = CopyOptions {
        metadata_set: vec![("title".into(), "MP4 Edited Wave Title".into())],
        metadata_delete: vec!["comment".into()],
        ..Default::default()
    };
    let out_mp4_edited = tmp.0.join("mp4-decode-edited.wav");
    fvid::media::decode_audio_transformed(
        &mp4,
        &out_mp4_edited,
        AudioDecodeTransform::default(),
        &mp4_edit_opts,
    )
    .expect("decode_audio mp4 edited");
    let probed_mp4_edited = fvid::media::probe(&out_mp4_edited).unwrap();
    assert_eq!(
        probed_mp4_edited.metadata.get("title").map(String::as_str),
        Some("MP4 Edited Wave Title")
    );
    assert_eq!(
        probed_mp4_edited.metadata.get("artist").map(String::as_str),
        Some("MP4 Source Artist"),
        "untouched MP4 artist tag must be preserved when editing title/comment"
    );
    assert_eq!(
        probed_mp4_edited.metadata.get("album").map(String::as_str),
        Some("MP4 Source Album")
    );
    assert_eq!(
        probed_mp4_edited.metadata.get("ILNG").map(String::as_str),
        Some("fra")
    );
    assert!(!probed_mp4_edited.metadata.contains_key("comment"));

    // MP4 `trim` to .wav (both default and with metadata edits)
    let out_mp4_trim = tmp.0.join("mp4-trim-default.wav");
    fvid::media::trim(&mp4, &out_mp4_trim, 0, 250_000, &CopyOptions::default())
        .expect("trim mp4 default");
    let probed_mp4_trim = fvid::media::probe(&out_mp4_trim).unwrap();
    assert_eq!(
        probed_mp4_trim.metadata.get("title").map(String::as_str),
        Some("MP4 Source Title")
    );
    assert_eq!(
        probed_mp4_trim.metadata.get("artist").map(String::as_str),
        Some("MP4 Source Artist")
    );

    // MP4 `loudnorm` to .wav (both default and with metadata edits)
    let out_mp4_ln = tmp.0.join("mp4-loudnorm-default.wav");
    fvid::native_loudnorm::try_apply(&mp4, &out_mp4_ln, None, false, &CopyOptions::default())
        .expect("loudnorm mp4 default")
        .expect("loudnorm mp4 handled");
    let probed_mp4_ln = fvid::media::probe(&out_mp4_ln).unwrap();
    assert_eq!(
        probed_mp4_ln.metadata.get("title").map(String::as_str),
        Some("MP4 Source Title")
    );
    assert_eq!(
        probed_mp4_ln.metadata.get("artist").map(String::as_str),
        Some("MP4 Source Artist")
    );

    let out_mp4_ln_edit = tmp.0.join("mp4-loudnorm-edited.wav");
    fvid::native_loudnorm::try_apply(&mp4, &out_mp4_ln_edit, None, true, &mp4_edit_opts)
        .expect("loudnorm mp4 edited")
        .expect("loudnorm mp4 edited handled");
    let probed_mp4_ln_edit = fvid::media::probe(&out_mp4_ln_edit).unwrap();
    assert_eq!(
        probed_mp4_ln_edit.metadata.get("title").map(String::as_str),
        Some("MP4 Edited Wave Title")
    );
    assert_eq!(
        probed_mp4_ln_edit
            .metadata
            .get("artist")
            .map(String::as_str),
        Some("MP4 Source Artist")
    );
    assert!(!probed_mp4_ln_edit.metadata.contains_key("comment"));

    // 3. Matroska -> .wav across `decode_audio`, `trim`, `loudnorm`, and `normalize-loudness` (API and CLI)
    //    must preserve source Matroska container & track tags in RIFF INFO and support selective override/delete.
    let out_mka_decode = tmp.0.join("mka-decode-default.wav");
    fvid::media::decode_audio(&mka, &out_mka_decode, &CopyOptions::default())
        .expect("decode_audio mka default");
    let probed_mka_decode = fvid::media::probe(&out_mka_decode).unwrap();
    assert_eq!(
        probed_mka_decode.metadata.get("title").map(String::as_str),
        Some("MKA Source Title")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("artist").map(String::as_str),
        Some("MKA Source Artist")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("album").map(String::as_str),
        Some("MKA Source Album")
    );
    assert_eq!(
        probed_mka_decode
            .metadata
            .get("comment")
            .map(String::as_str),
        Some("MKA Source Comment")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("date").map(String::as_str),
        Some("2025")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("genre").map(String::as_str),
        Some("Synth")
    );
    assert_eq!(
        probed_mka_decode
            .metadata
            .get("copyright")
            .map(String::as_str),
        Some("MKA Source Copyright")
    );
    assert_eq!(
        probed_mka_decode
            .metadata
            .get("encoder")
            .map(String::as_str),
        Some("FVidSynthMuxer")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("track").map(String::as_str),
        Some("3/9")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("ILNG").map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("QWER").map(String::as_str),
        Some("MkaCustomFourCC")
    );
    assert_eq!(
        probed_mka_decode.metadata.get("ISRC").map(String::as_str),
        Some("US-FVD-26-00001")
    );

    // CLI `decode-audio`, `trim`, `loudnorm`, and `normalize-loudness` on Matroska -> .wav with selective metadata edits
    let cli_mka_trim_wav = tmp.0.join("cli-mka-trim.wav");
    let cli_trim = Command::new(bin_path())
        .args([
            "media",
            "trim",
            mka.to_str().unwrap(),
            cli_mka_trim_wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.25",
            "--metadata",
            "title=CLI Trimmed MKA",
            "--metadata-delete",
            "comment",
            "-q",
        ])
        .output()
        .expect("cli trim mka to wav");
    assert!(
        cli_trim.status.success(),
        "cli trim mka to wav failed: {}",
        String::from_utf8_lossy(&cli_trim.stderr)
    );
    let probed_cli_trim = fvid::media::probe(&cli_mka_trim_wav).unwrap();
    assert_eq!(
        probed_cli_trim.metadata.get("title").map(String::as_str),
        Some("CLI Trimmed MKA")
    );
    assert_eq!(
        probed_cli_trim.metadata.get("artist").map(String::as_str),
        Some("MKA Source Artist")
    );
    assert_eq!(
        probed_cli_trim.metadata.get("ISRC").map(String::as_str),
        Some("US-FVD-26-00001")
    );
    assert!(!probed_cli_trim.metadata.contains_key("comment"));

    let cli_mka_norm_wav = tmp.0.join("cli-mka-norm.wav");
    let cli_norm = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            mka.to_str().unwrap(),
            cli_mka_norm_wav.to_str().unwrap(),
            "--metadata",
            "artist=CLI Normalized Artist",
            "--metadata-delete",
            "QWER",
            "-q",
        ])
        .output()
        .expect("cli normalize-loudness mka to wav");
    assert!(
        cli_norm.status.success(),
        "cli normalize-loudness mka to wav failed: {}",
        String::from_utf8_lossy(&cli_norm.stderr)
    );
    let probed_cli_norm = fvid::media::probe(&cli_mka_norm_wav).unwrap();
    assert_eq!(
        probed_cli_norm.metadata.get("title").map(String::as_str),
        Some("MKA Source Title")
    );
    assert_eq!(
        probed_cli_norm.metadata.get("artist").map(String::as_str),
        Some("CLI Normalized Artist")
    );
    assert!(!probed_cli_norm.metadata.contains_key("QWER"));
}
