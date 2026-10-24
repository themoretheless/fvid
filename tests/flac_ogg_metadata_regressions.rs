//! Regression tests for Vorbis comment metadata inspection and WAVE INFO preservation
//! across bare FLAC (`probe`, `decode-audio`, `trim`, `loudnorm`) and Ogg (`probe`) (FVID-0229).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use fvid::media::{
    apply_loudnorm, apply_loudnorm_dual, decode_audio_transformed, probe, trim,
    AudioDecodeTransform, CopyOptions,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-ogg-metadata")
        .join(name)
}

fn temp_out(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "fvid-flac-ogg-metadata-{label}-{}-{id}.wav",
        std::process::id()
    ))
}

fn assert_common_vorbis_metadata(info: &fvid::media::MediaInfo) {
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Synthetic Tone")
    );
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("FVid Lab")
    );
    assert_eq!(
        info.metadata.get("album").map(String::as_str),
        Some("Regression Suite")
    );
    assert_eq!(
        info.metadata.get("date").map(String::as_str),
        Some("2026-10-23")
    );
    assert_eq!(
        info.metadata.get("comment").map(String::as_str),
        Some("Synthetic Vorbis Comment Fixture")
    );
    assert_eq!(
        info.metadata.get("genre").map(String::as_str),
        Some("Test")
    );
    assert_eq!(
        info.metadata.get("copyright").map(String::as_str),
        Some("Public Domain")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-synthetic-generator")
    );
    assert_eq!(
        info.metadata.get("track").map(String::as_str),
        Some("3/12")
    );
    assert_eq!(
        info.metadata.get("album_artist").map(String::as_str),
        Some("FVid Ensemble")
    );
    assert_eq!(
        info.metadata.get("disc").map(String::as_str),
        Some("1/2")
    );
    assert_eq!(
        info.metadata.get("publisher").map(String::as_str),
        Some("FVid Press")
    );
    assert_eq!(
        info.metadata.get("description").map(String::as_str),
        Some("Lossless metadata verification")
    );
    assert_eq!(
        info.metadata.get("rating").map(String::as_str),
        Some("5")
    );
    assert_eq!(
        info.metadata.get("language").map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        info.metadata.get("custom_tag").map(String::as_str),
        Some("custom-value")
    );
    assert!(!info
        .metadata
        .contains_key("waveformatextensible_channel_mask"));
    assert_eq!(
        info.streams[0].metadata.get("title").map(String::as_str),
        Some("Synthetic Tone")
    );
    assert_eq!(
        info.streams[0].metadata.get("language").map(String::as_str),
        Some("eng")
    );
}

#[test]
fn bare_flac_and_ogg_probes_expose_vorbis_comment_metadata() {
    for (file, expected_format) in [
        ("tagged.flac", "flac"),
        ("tagged-opus.ogg", "ogg"),
        ("tagged-vorbis.ogg", "ogg"),
    ] {
        let path = fixture_path(file);
        let top = probe(&path).expect("top-level probe should succeed");
        assert_eq!(top.format, expected_format);
        assert_common_vorbis_metadata(&top);

        let owned = fvid_media::probe(&path).expect("owned library probe should succeed");
        assert_eq!(owned.format, expected_format);
        assert_common_vorbis_metadata(&owned);
    }
}

#[test]
fn bare_flac_wave_exports_and_loudnorm_preserve_and_edit_source_vorbis_comments() {
    let flac = fixture_path("tagged.flac");

    // 1. decode-audio preserves source Vorbis comments as RIFF INFO tags and supports selective delete/override.
    let decoded_wav = temp_out("decode");
    let _ = std::fs::remove_file(&decoded_wav);
    let options = CopyOptions {
        metadata_set: vec![("title".into(), "Overridden Title".into())],
        metadata_delete: vec!["comment".into()],
        ..CopyOptions::default()
    };
    decode_audio_transformed(
        &flac,
        &decoded_wav,
        AudioDecodeTransform::default(),
        &options,
    )
    .expect("decode-audio from tagged FLAC should succeed");
    let info = probe(&decoded_wav).expect("probe decoded WAV");
    let _ = std::fs::remove_file(&decoded_wav);
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Overridden Title")
    );
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("FVid Lab")
    );
    assert_eq!(
        info.metadata.get("album").map(String::as_str),
        Some("Regression Suite")
    );
    assert_eq!(
        info.metadata.get("date").map(String::as_str),
        Some("2026-10-23")
    );
    assert!(!info.metadata.contains_key("comment"));
    assert_eq!(
        info.metadata.get("genre").map(String::as_str),
        Some("Test")
    );
    assert_eq!(
        info.metadata.get("copyright").map(String::as_str),
        Some("Public Domain")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-synthetic-generator")
    );
    assert_eq!(
        info.metadata.get("track").map(String::as_str),
        Some("3/12")
    );
    assert_eq!(info.metadata.get("ILNG").map(String::as_str), Some("eng"));

    // 2. trim input.flac output.wav preserves source Vorbis comments and applies metadata edits.
    let trimmed_wav = temp_out("trim");
    let _ = std::fs::remove_file(&trimmed_wav);
    trim(&flac, &trimmed_wav, 0, 50_000, &options)
        .expect("trim from tagged FLAC to WAV should succeed");
    let trimmed_info = probe(&trimmed_wav).expect("probe trimmed WAV");
    let _ = std::fs::remove_file(&trimmed_wav);
    assert_eq!(
        trimmed_info.metadata.get("title").map(String::as_str),
        Some("Overridden Title")
    );
    assert_eq!(
        trimmed_info.metadata.get("artist").map(String::as_str),
        Some("FVid Lab")
    );
    assert!(!trimmed_info.metadata.contains_key("comment"));
    assert_eq!(
        trimmed_info.metadata.get("ILNG").map(String::as_str),
        Some("eng")
    );

    // 3. single-pass and dual-pass loudnorm input.flac output.wav preserve source Vorbis comments and apply metadata edits.
    for (label, dual_pass) in [("loudnorm-single", false), ("loudnorm-dual", true)] {
        let norm_wav = temp_out(label);
        let _ = std::fs::remove_file(&norm_wav);
        if dual_pass {
            apply_loudnorm_dual(&flac, &norm_wav, None, &options)
                .expect("two-pass loudnorm from tagged FLAC should succeed");
        } else {
            apply_loudnorm(&flac, &norm_wav, None, &options)
                .expect("single-pass loudnorm from tagged FLAC should succeed");
        }
        let norm_info = probe(&norm_wav).expect("probe normalized WAV");
        let _ = std::fs::remove_file(&norm_wav);
        assert_eq!(
            norm_info.metadata.get("title").map(String::as_str),
            Some("Overridden Title")
        );
        assert_eq!(
            norm_info.metadata.get("artist").map(String::as_str),
            Some("FVid Lab")
        );
        assert_eq!(
            norm_info.metadata.get("album").map(String::as_str),
            Some("Regression Suite")
        );
        assert!(!norm_info.metadata.contains_key("comment"));
        assert_eq!(
            norm_info.metadata.get("track").map(String::as_str),
            Some("3/12")
        );
        assert_eq!(
            norm_info.metadata.get("ILNG").map(String::as_str),
            Some("eng")
        );
    }
}
