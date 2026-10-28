use fvid::{
    container::{mp4::Mp4Reader, mp4_concat},
    media::CopyOptions,
};
use std::{fs::File, io::BufReader, path::PathBuf};
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-discarded-sync/unmarked-video-sync.mp4")
}
#[test]
fn selected_audio_does_not_require_discarded_video_sync() {
    let source = source();
    let reader = Mp4Reader::open(
        BufReader::new(File::open(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(reader.tracks().len(), 2);
    assert!(!reader.tracks()[0].samples.get(0).unwrap().sync);
    assert_eq!(reader.tracks()[1].samples.len(), 2);
    let options = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    assert!(
        mp4_concat::open_with_options(&[source.clone(), source], &options)
            .unwrap()
            .is_some(),
        "unselected video random access requirements must not reject selected audio"
    );
}
#[test]
fn selected_video_without_declared_sync_still_refuses() {
    for streams in [vec![], vec![0]] {
        let source = source();
        let options = CopyOptions {
            streams,
            ..Default::default()
        };
        assert!(
            mp4_concat::open_with_options(&[source.clone(), source], &options)
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn selected_audio_public_export_preserves_packets_and_decodes_full_pcm() {
    use fvid::container::webm::WebmReader;
    let source = source();
    let options = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let dir = std::env::temp_dir().join(format!("fvid-discarded-sync-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let original = fvid::media::decode_audio(&source, &dir.join("source.wav"), &options).unwrap();
    assert_eq!(original.sample_frames, 2048);
    let inputs = [source.clone(), source.clone()];
    let plan = fvid::media::plan_concat(&inputs, &options).unwrap();
    assert_eq!(plan.streams.len(), 1);
    let output = dir.join("out.mka");
    assert_eq!(
        fvid::media::concat(&inputs, &output, &options)
            .unwrap()
            .packets,
        4
    );
    let mut input = Mp4Reader::open(
        BufReader::new(File::open(source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mut result = WebmReader::open(
        BufReader::new(File::open(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    result.scan_all().unwrap();
    assert_eq!(result.tracks.len(), 1);
    assert_eq!(result.packets.len(), 4);
    let mut packet = Vec::new();
    for index in 0..4 {
        input.read_packet(1, index % 2, &mut packet).unwrap();
        assert_eq!(result.read_packet(index).unwrap(), packet);
    }
    let decoded =
        fvid::media::decode_audio(&output, &dir.join("output.wav"), &CopyOptions::default())
            .unwrap();
    assert_eq!(decoded.sample_rate, 48000);
    assert_eq!(decoded.channels, 1);
    assert_eq!(decoded.sample_frames, 4096);
}
