use fvid::{
    container::{
        mp4::{Limits, Mp4Reader},
        mp4_concat,
    },
    media::CopyOptions,
};
use std::{fs::File, io::BufReader, path::PathBuf};
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-retained-plans/two-tracks.m4a")
}
#[test]
fn selected_track_does_not_hide_retained_plan_memory() {
    let source = source();
    let reader = Mp4Reader::open(
        BufReader::new(File::open(&source).unwrap()),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(reader.tracks().len(), 2);
    assert_eq!(reader.tracks()[0].samples.len(), 2);
    assert_eq!(reader.tracks()[1].samples.len(), 512);
    let options = CopyOptions {
        streams: vec![0],
        max_controlled_bytes: Some(40 << 10),
        ..Default::default()
    };
    let result = mp4_concat::open_with_options(&[source.clone(), source], &options);
    assert!(
        matches!(result, Err(e) if e.to_string().contains("controlled memory budget exceeded")),
        "unselected track plans must count while retained"
    );
}
#[test]
fn generous_budget_accepts_the_same_selected_track() {
    let source = source();
    let options = CopyOptions {
        streams: vec![0],
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    assert!(
        mp4_concat::open_with_options(&[source.clone(), source], &options)
            .unwrap()
            .is_some()
    );
}
#[test]
fn generous_budget_writes_only_selected_packets_and_decodes_audio() {
    use std::io::{Cursor, Read};
    let source = source();
    let options = CopyOptions {
        streams: vec![0],
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    let mut segments = mp4_concat::open_with_options(&[source.clone(), source], &options)
        .unwrap()
        .unwrap();
    let mut output = Cursor::new(Vec::new());
    let stats = mp4_concat::write_with_options(&mut segments, &mut output, &options).unwrap();
    assert_eq!(stats.packets, 4);
    let dir = std::env::temp_dir().join(format!("fvid-concat-retained-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let encoded = dir.join("out.mka");
    let decoded = dir.join("out.wav");
    std::fs::write(&encoded, output.into_inner()).unwrap();
    fvid::media::decode_audio(&encoded, &decoded, &CopyOptions::default()).unwrap();
    let mut pcm = Vec::new();
    File::open(decoded).unwrap().read_to_end(&mut pcm).unwrap();
    assert!(pcm.starts_with(b"RIFF"));
    assert!(
        pcm.len() >= 44 + 4096 * 2,
        "four AAC frames must decode completely"
    );
}
