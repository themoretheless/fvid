//! MP4 presentation edits feed owned filtering and atomic lossless export.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn raw_frames(path: &Path) -> Vec<Vec<u8>> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        frames.push(match frame {
            fvid::playback_native::RawFrame::Planar(frame) => frame.frame.data.clone(),
            fvid::playback_native::RawFrame::Planar8(frame) => {
                [frame.y.as_slice(), frame.cb.as_slice(), frame.cr.as_slice()].concat()
            }
            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                let mut data = Vec::new();
                picture.write_planar(&mut data).unwrap();
                data
            }
            _ => panic!("expected planar compressed-video output"),
        });
    }
    frames
}

#[test]
fn mp4_presentation_pixels_survive_owned_filters_and_export() {
    for (case, (name, depth)) in [
        ("playback-errors/shared-avc-baseline.mp4", 8),
        ("playback-errors/shared-avc-bframes.mp4", 8),
        ("playback-errors/shared-hevc-main.mp4", 8),
        ("playback-errors/shared-hevc-main10.mp4", 10),
        ("playback-errors/shared-edit-repeat.mp4", 8),
        ("playback-errors/shared-edit-disjoint.mp4", 8),
        ("playback-errors/shared-edit-fractional.mp4", 8),
        ("playback-errors/shared-edit-leading.mp4", 10),
        ("playback-errors/duplicate-pts.mp4", 8),
        ("playback-errors/duplicate-pts-run.mp4", 8),
        ("playback-errors/duplicate-pts-all.mp4", 8),
    ]
    .into_iter()
    .enumerate()
    {
        let source = fixture(name);
        let originals = raw_frames(&source);
        for (operation, request, expected) in [
            (
                "plain",
                fvid_media::LosslessTransform::default(),
                originals.clone(),
            ),
            (
                "negate",
                fvid_media::LosslessTransform {
                    negate: Some("".into()),
                    ..Default::default()
                },
                originals
                    .iter()
                    .map(|bytes| {
                        if depth == 8 {
                            bytes.iter().map(|v| 255 - v).collect()
                        } else {
                            bytes
                                .as_chunks::<2>().0.iter()
                                .flat_map(|v| {
                                    (((1u16 << depth) - 1) - u16::from_le_bytes([v[0], v[1]]))
                                        .to_le_bytes()
                                })
                                .collect()
                        }
                    })
                    .collect(),
            ),
            (
                "step",
                fvid_media::LosslessTransform {
                    framestep: Some("2".into()),
                    ..Default::default()
                },
                originals.iter().step_by(2).cloned().collect(),
            ),
            (
                "reverse",
                fvid_media::LosslessTransform {
                    reverse: Some("".into()),
                    ..Default::default()
                },
                originals.iter().rev().cloned().collect(),
            ),
        ] {
            let output = std::env::temp_dir().join(format!(
                "fvid-mp4-pipeline-{}-{case}-{operation}.mkv",
                std::process::id()
            ));
            let stats =
                fvid_media::transcode_lossless(&source, &output, request, &Default::default())
                    .unwrap_or_else(|e| panic!("{name}/{operation}: {e}"));
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.video_frames, expected.len() as u64);
            let actual = raw_frames(&output);
            assert!(
                actual == expected,
                "{name}/{operation}: pixel mismatch, actual {} frames, expected {}",
                actual.len(),
                expected.len()
            );
            if operation == "plain" {
                let mut native = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(&source).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                let mut timeline = Vec::new();
                while native.read_frame_raw().unwrap().is_some() {
                    let (start, end, scale) = native.frame_interval().unwrap();
                    let start = u64::try_from(start * 1_000_000_000 / u128::from(scale)).unwrap();
                    let end = u64::try_from(end * 1_000_000_000 / u128::from(scale)).unwrap();
                    timeline.push((start as i64, Some(end - start)));
                }
                let mut reader = fvid_media::owned_webm::WebmReader::open(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    Default::default(),
                )
                .unwrap();
                reader.scan_all().unwrap();
                assert_eq!(
                    reader
                        .packets
                        .iter()
                        .map(|p| (p.pts_ns, p.duration_ns))
                        .collect::<Vec<_>>(),
                    timeline,
                    "{name}"
                );
            }
            std::fs::remove_file(output).unwrap();
        }
        let decoded = fvid_media::decode_video_transformed(
            &source,
            fvid_media::DecodeTransform {
                negate: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(decoded.backend, "owned MP4 compressed video pipeline");
        assert_eq!(decoded.video_frames, originals.len() as u64);
    }
}

#[test]
fn undefined_mp4_language_maps_to_und_without_rejecting_owned_export() {
    let source = fixture("playback-errors/shared-mp4-undefined-language.mp4");
    let reader = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert!(reader.tracks()[0].language.is_empty());
    let output = std::env::temp_dir().join(format!("fvid-mp4-und-{}.mkv", std::process::id()));
    fvid_media::transcode_lossless(&source, &output, Default::default(), &Default::default())
        .unwrap();
    let mut decoded = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&output).unwrap()),
        Default::default(),
    )
    .unwrap();
    decoded.scan_all().unwrap();
    assert_eq!(
        decoded.track_languages.get(&1).map(String::as_str),
        Some("und")
    );
    assert_eq!(raw_frames(&output), raw_frames(&source));
    std::fs::remove_file(output).unwrap();
}

#[test]
fn admitted_mp4_packet_corruption_stays_owned_and_atomic() {
    let source = fixture("playback-errors/shared-mp4-corrupt-nal.mp4");
    let mut reader = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mut packet = Vec::new();
    reader.read_packet(0, 0, &mut packet).unwrap();
    assert_eq!(&packet[..4], &[255; 4]);
    assert!(fvid_media::owned_lossless::supports(
        &source,
        &Default::default(),
        &Default::default()
    ));
    let output = std::env::temp_dir().join(format!("fvid-mp4-corrupt-{}.mkv", std::process::id()));
    let error =
        fvid_media::transcode_lossless(&source, &output, Default::default(), &Default::default())
            .unwrap_err();
    assert!(
        error.to_string().contains("invalid NAL payload length"),
        "{error}"
    );
    assert!(!output.exists());
}

#[test]
fn mp4_audio_track_does_not_block_owned_video_filters_or_disappear_on_export() {
    let source = fixture("playback-errors/shared-mp4-av.mp4");
    let reader = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(reader.tracks().len(), 2);
    assert_eq!(reader.tracks()[0].handler, *b"vide");
    assert_eq!(reader.tracks()[1].handler, *b"soun");
    assert_eq!(reader.tracks()[1].codec, *b"mp4a");
    assert!(!reader.tracks()[1].samples.is_empty());
    let request = fvid_media::DecodeTransform {
        negate: Some("".into()),
        ..Default::default()
    };
    let stats = fvid_media::decode_video_transformed(&source, request.clone()).unwrap();
    assert_eq!(stats.backend, "owned MP4 compressed video pipeline");
    let control = fvid_media::decode_video_transformed(
        &fixture("playback-errors/shared-avc-baseline.mp4"),
        request,
    )
    .unwrap();
    assert_eq!(stats.video_frames, control.video_frames);
    assert_eq!(
        raw_frames(&source),
        raw_frames(&fixture("playback-errors/shared-avc-baseline.mp4"))
    );
    assert!(fvid_media::owned_lossless::supports(
        &source,
        &Default::default(),
        &Default::default()
    ));
}
