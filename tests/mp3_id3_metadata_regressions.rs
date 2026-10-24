//! Regression tests for ID3v2/ID3v1 metadata inspection and WAVE INFO preservation
//! across bare MP3 (`probe`, `decode-audio`, `trim`, `loudnorm`) (FVID-0230).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use fvid::media::{
    apply_loudnorm, apply_loudnorm_dual, decode_audio_transformed, probe, trim,
    AudioDecodeTransform, CopyOptions,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-id3-metadata")
        .join(name)
}

fn temp_out(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "fvid-mp3-id3-metadata-{label}-{}-{id}.wav",
        std::process::id()
    ))
}

fn assert_id3v24_metadata(info: &fvid::media::MediaInfo) {
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Synthetic MP3 Tone")
    );
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("FVid Audio Lab")
    );
    assert_eq!(
        info.metadata.get("album").map(String::as_str),
        Some("MPEG Metadata Suite")
    );
    assert_eq!(
        info.metadata.get("date").map(String::as_str),
        Some("2026-10-23")
    );
    assert_eq!(
        info.metadata.get("comment").map(String::as_str),
        Some("Synthetic ID3v2.4 Comment")
    );
    assert_eq!(
        info.metadata.get("genre").map(String::as_str),
        Some("Electronic")
    );
    assert_eq!(
        info.metadata.get("copyright").map(String::as_str),
        Some("Public Domain")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-synthetic-mp3")
    );
    assert_eq!(
        info.metadata.get("track").map(String::as_str),
        Some("4/10")
    );
    assert_eq!(
        info.metadata.get("album_artist").map(String::as_str),
        Some("FVid Orchestra")
    );
    assert_eq!(
        info.metadata.get("disc").map(String::as_str),
        Some("1/2")
    );
    assert_eq!(
        info.metadata.get("publisher").map(String::as_str),
        Some("FVid Records")
    );
    assert_eq!(
        info.metadata.get("description").map(String::as_str),
        Some("Lossless ID3 verification")
    );
    assert_eq!(
        info.metadata.get("language").map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        info.metadata.get("custom_tag").map(String::as_str),
        Some("custom-mp3-value")
    );
    assert_eq!(
        info.streams[0].metadata.get("title").map(String::as_str),
        Some("Synthetic MP3 Tone")
    );
    assert_eq!(
        info.streams[0].metadata.get("language").map(String::as_str),
        Some("eng")
    );
}

#[test]
fn bare_mp3_probes_expose_id3v2_and_id3v1_metadata() {
    let v24_path = fixture_path("tagged-id3v24.mp3");
    let top_v24 = probe(&v24_path).expect("top-level probe should succeed for ID3v2.4 MP3");
    assert_eq!(top_v24.format, "mp3");
    assert_id3v24_metadata(&top_v24);

    let owned_v24 =
        fvid_media::probe(&v24_path).expect("owned library probe should succeed for ID3v2.4 MP3");
    assert_eq!(owned_v24.format, "mp3");
    assert_id3v24_metadata(&owned_v24);

    let v1_path = fixture_path("tagged-id3v1.mp3");
    for info in [
        probe(&v1_path).expect("top-level probe should succeed for ID3v1 MP3"),
        fvid_media::probe(&v1_path).expect("owned library probe should succeed for ID3v1 MP3"),
    ] {
        assert_eq!(info.format, "mp3");
        assert_eq!(
            info.metadata.get("title").map(String::as_str),
            Some("ID3v1 Fallback Title")
        );
        assert_eq!(
            info.metadata.get("artist").map(String::as_str),
            Some("ID3v1 Artist")
        );
        assert_eq!(
            info.metadata.get("album").map(String::as_str),
            Some("ID3v1 Album")
        );
        assert_eq!(
            info.metadata.get("date").map(String::as_str),
            Some("2025")
        );
        assert_eq!(
            info.metadata.get("comment").map(String::as_str),
            Some("ID3v1 Comment")
        );
        assert_eq!(
            info.metadata.get("track").map(String::as_str),
            Some("7")
        );
        assert_eq!(
            info.metadata.get("genre").map(String::as_str),
            Some("Other")
        );
        assert_eq!(
            info.streams[0].metadata.get("title").map(String::as_str),
            Some("ID3v1 Fallback Title")
        );
    }
}

#[test]
fn bare_mp3_wave_exports_and_loudnorm_preserve_and_edit_source_id3_tags() {
    let mp3 = fixture_path("tagged-id3v24.mp3");

    // 1. decode-audio preserves source ID3 tags as RIFF INFO tags and supports selective delete/override.
    let decoded_wav = temp_out("decode");
    let _ = std::fs::remove_file(&decoded_wav);
    let options = CopyOptions {
        metadata_set: vec![("title".into(), "Overridden MP3 Title".into())],
        metadata_delete: vec!["comment".into()],
        ..CopyOptions::default()
    };
    decode_audio_transformed(
        &mp3,
        &decoded_wav,
        AudioDecodeTransform::default(),
        &options,
    )
    .expect("decode-audio from tagged MP3 should succeed");
    let info = probe(&decoded_wav).expect("probe decoded WAV");
    let _ = std::fs::remove_file(&decoded_wav);
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Overridden MP3 Title")
    );
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("FVid Audio Lab")
    );
    assert_eq!(
        info.metadata.get("album").map(String::as_str),
        Some("MPEG Metadata Suite")
    );
    assert_eq!(
        info.metadata.get("date").map(String::as_str),
        Some("2026-10-23")
    );
    assert!(!info.metadata.contains_key("comment"));
    assert_eq!(
        info.metadata.get("genre").map(String::as_str),
        Some("Electronic")
    );
    assert_eq!(
        info.metadata.get("copyright").map(String::as_str),
        Some("Public Domain")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-synthetic-mp3")
    );
    assert_eq!(
        info.metadata.get("track").map(String::as_str),
        Some("4/10")
    );
    assert_eq!(info.metadata.get("ILNG").map(String::as_str), Some("eng"));

    // 2. trim input.mp3 output.wav preserves source ID3 tags and applies metadata edits.
    let trimmed_wav = temp_out("trim");
    let _ = std::fs::remove_file(&trimmed_wav);
    trim(&mp3, &trimmed_wav, 0, 50_000, &options)
        .expect("trim from tagged MP3 to WAV should succeed");
    let trimmed_info = probe(&trimmed_wav).expect("probe trimmed WAV");
    let _ = std::fs::remove_file(&trimmed_wav);
    assert_eq!(
        trimmed_info.metadata.get("title").map(String::as_str),
        Some("Overridden MP3 Title")
    );
    assert_eq!(
        trimmed_info.metadata.get("artist").map(String::as_str),
        Some("FVid Audio Lab")
    );
    assert!(!trimmed_info.metadata.contains_key("comment"));
    assert_eq!(
        trimmed_info.metadata.get("ILNG").map(String::as_str),
        Some("eng")
    );

    // 3. single-pass and dual-pass loudnorm input.mp3 output.wav preserve source ID3 tags and apply metadata edits.
    for (label, dual_pass) in [("loudnorm-single", false), ("loudnorm-dual", true)] {
        let norm_wav = temp_out(label);
        let _ = std::fs::remove_file(&norm_wav);
        if dual_pass {
            apply_loudnorm_dual(&mp3, &norm_wav, None, &options)
                .expect("two-pass loudnorm from tagged MP3 should succeed");
        } else {
            apply_loudnorm(&mp3, &norm_wav, None, &options)
                .expect("single-pass loudnorm from tagged MP3 should succeed");
        }
        let norm_info = probe(&norm_wav).expect("probe normalized WAV");
        let _ = std::fs::remove_file(&norm_wav);
        assert_eq!(
            norm_info.metadata.get("title").map(String::as_str),
            Some("Overridden MP3 Title")
        );
        assert_eq!(
            norm_info.metadata.get("artist").map(String::as_str),
            Some("FVid Audio Lab")
        );
        assert_eq!(
            norm_info.metadata.get("album").map(String::as_str),
            Some("MPEG Metadata Suite")
        );
        assert!(!norm_info.metadata.contains_key("comment"));
        assert_eq!(
            norm_info.metadata.get("track").map(String::as_str),
            Some("4/10")
        );
        assert_eq!(
            norm_info.metadata.get("ILNG").map(String::as_str),
            Some("eng")
        );
    }
}
