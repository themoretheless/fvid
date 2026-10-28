use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::PathBuf;
fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-metadata-admission")
        .join(name)
}
#[test]
fn audio_geometry_respects_parser_metadata_ceiling() {
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error =
        fvid_media::owned_audio_plan::source_geometry(&source("padded.m4a"), &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn audio_decode_plan_respects_parser_metadata_ceiling() {
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error = fvid::media::plan_decode_audio(
        &source("padded.m4a"),
        &AudioDecodeTransform::default(),
        &options,
    )
    .unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn small_and_ample_budget_audio_controls_are_valid() {
    for (name, budget) in [("small.m4a", 1 << 20), ("padded.m4a", 8 << 20)] {
        let options = CopyOptions {
            max_controlled_bytes: Some(budget),
            ..Default::default()
        };
        assert_eq!(
            fvid_media::owned_audio_plan::source_geometry(&source(name), &options).unwrap(),
            (8000, 1)
        );
        assert!(
            fvid::media::plan_decode_audio(
                &source(name),
                &AudioDecodeTransform::default(),
                &options
            )
            .is_ok()
        );
    }
}
#[test]
fn native_probes_respect_the_same_metadata_ceiling() {
    let path = source("padded.m4a");
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error =
        fvid::native_media::is_aac_source_with_metadata_limit(&path, options.max_controlled_bytes)
            .unwrap_err();
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
    let error =
        match fvid::native_media::audio_source_info_selected_with_options(&path, None, &options) {
            Ok(_) => panic!("native audio probe must refuse oversized metadata"),
            Err(error) => error,
        };
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
}
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("fvid-audio-metadata-{name}-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn decode_refuses_before_output_and_leaves_no_temporary_files() {
    let dir = TempDir::new("refusal");
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    for route in 0..2 {
        let output = dir.0.join(format!("out-{route}.wav"));
        let result = if route == 0 {
            fvid::media::decode_audio(&source("padded.m4a"), &output, &options)
        } else {
            fvid_media::decode_audio(&source("padded.m4a"), &output, &options)
        };
        let error = result.unwrap_err();
        assert!(error.contains("metadata exceeds budget"), "{error}");
        assert!(!output.exists());
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
}
fn wave_chunks(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut at = 12;
    let mut format = None;
    let mut pcm = None;
    while at + 8 <= data.len() {
        let size = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &data[at + 8..at + 8 + size];
        match &data[at..at + 4] {
            b"fmt " => format = Some(body.to_vec()),
            b"data" => pcm = Some(body.to_vec()),
            _ => {}
        }
        at += 8 + size + (size % 2);
    }
    (format.unwrap(), pcm.unwrap())
}
#[test]
fn ample_budget_decodes_exactly_the_same_pcm_as_unpadded_input() {
    let dir = TempDir::new("acceptance");
    let options = CopyOptions {
        max_controlled_bytes: Some(8 << 20),
        ..Default::default()
    };
    let mut reference = None;
    for name in ["small.m4a", "padded.m4a"] {
        let output = dir.0.join(format!("{name}.wav"));
        let stats = fvid::media::decode_audio(&source(name), &output, &options).unwrap();
        assert_eq!(stats.sample_frames, 136);
        assert_eq!(stats.sample_rate, 8000);
        assert_eq!(stats.channels, 1);
        let chunks = wave_chunks(&std::fs::read(output).unwrap());
        assert!(!chunks.1.is_empty());
        if let Some(expected) = &reference {
            assert_eq!(&chunks, expected);
        } else {
            reference = Some(chunks);
        }
    }
}
