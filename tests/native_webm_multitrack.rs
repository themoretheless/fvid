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

fn packets(
    reader: &mut fvid_media::owned_webm::WebmReader<Cursor<Vec<u8>>>,
    track: u64,
) -> Vec<(i64, Option<u64>, i64, Vec<u8>)> {
    let mut out = Vec::new();
    for i in 0..reader.packets.len() {
        let p = reader.packets[i].clone();
        if p.track == track {
            out.push((
                p.pts_ns,
                p.duration_ns,
                p.discard_padding_ns,
                reader.read_packet(i).unwrap(),
            ));
        }
    }
    out
}
fn wave_payload(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let count = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            return bytes[at + 8..at + 8 + count].to_vec();
        }
        at += 8 + count + (count & 1);
    }
    panic!("missing PCM data")
}

fn opus_pcm(
    reader: &mut fvid_media::owned_webm::WebmReader<Cursor<Vec<u8>>>,
    index: usize,
) -> Vec<u8> {
    let track = reader.tracks[index].clone();
    let header = fvid_opus::OpusHead::parse(&track.codec_private).unwrap();
    let mut decoder = header.decoder(48_000).unwrap();
    let mut scratch = vec![0.0; 5760 * usize::from(header.channel_count)];
    let mut pcm = Vec::new();
    for packet in packets(reader, track.number) {
        let frames = decoder.decode(&packet.3, 5760, &mut scratch).unwrap();
        for sample in &scratch[..frames * usize::from(header.channel_count)] {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
    }
    assert!(!pcm.is_empty());
    pcm
}
#[test]
fn compressed_webm_filters_copy_aac_and_opus_without_changing_their_timeline() {
    for codec in ["vp9", "av1", "vp9-untimed"] {
        let source = fixture(&format!("playback-errors/shared-{codec}-companions.mkv"));
        let mut input = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        input.scan_all().unwrap();
        assert_eq!(
            input
                .tracks
                .iter()
                .map(|t| t.codec.as_str())
                .collect::<Vec<_>>(),
            vec![
                "A_AAC",
                if codec == "av1" { "V_AV1" } else { "V_VP9" },
                "A_OPUS"
            ]
        );
        let original = raw_frames(&source);
        assert!(!original.is_empty());
        let mut aac = packets(&mut input, 2);
        let mut opus = packets(&mut input, 3);
        let config =
            fvid_media::owned_aac::config::AacConfig::parse(&input.tracks[0].codec_private)
                .unwrap();
        for p in &mut aac {
            if p.1.is_none() {
                p.1 = Some(
                    (u64::from(config.frame_samples) * 1_000_000_000)
                        .div_ceil(u64::from(config.sample_rate)),
                );
            }
        }
        for p in &mut opus {
            if p.1.is_none() {
                p.1 = Some(fvid_media::owned_opus_packet::duration_ns(&p.3).unwrap());
            }
        }

        assert!(!aac.is_empty() && !opus.is_empty());
        let mut audio = Vec::new();
        for i in [0, 2] {
            if i == 2 {
                audio.push(opus_pcm(&mut input, i));
                continue;
            }

            let wav = std::env::temp_dir().join(format!(
                "fvid-webm-source-{}-{codec}-{i}.wav",
                std::process::id()
            ));
            let stats = fvid_media::decode_audio(
                &source,
                &wav,
                &fvid_control::CopyOptions {
                    streams: vec![i],
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(stats.sample_frames > 0);
            audio.push(wave_payload(&wav));
            std::fs::remove_file(wav).unwrap();
        }
        for (operation, transform, expected) in [
            (
                "negate",
                fvid_media::LosslessTransform {
                    negate: Some("".into()),
                    ..Default::default()
                },
                original
                    .iter()
                    .map(|f| f.iter().map(|p| 255 - p).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
            ),
            (
                "reverse",
                fvid_media::LosslessTransform {
                    reverse: Some("".into()),
                    ..Default::default()
                },
                original.iter().rev().cloned().collect(),
            ),
            (
                "step",
                fvid_media::LosslessTransform {
                    framestep: Some("2".into()),
                    ..Default::default()
                },
                original.iter().step_by(2).cloned().collect(),
            ),
        ] {
            for (case, selected) in [vec![0, 1, 2], vec![2, 1, 0], vec![1]]
                .into_iter()
                .enumerate()
            {
                let output = std::env::temp_dir().join(format!(
                    "fvid-webm-companions-{}-{codec}-{operation}-{case}.mkv",
                    std::process::id()
                ));
                let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let capture = events.clone();
                let published = output.clone();
                let mut options = fvid_control::CopyOptions {
                    streams: selected.clone(),
                    metadata_set: vec![("title".into(), "owned companions".into())],
                    ..Default::default()
                };
                for i in 0..3 {
                    options
                        .stream_metadata_set
                        .push((i, "title".into(), format!("source-{i}")));
                    options
                        .stream_metadata_set
                        .push((i, "role".into(), format!("role-{i}")));
                }
                options.progress = Some(fvid_control::ProgressHook::new(move |event| {
                    if event.done {
                        assert!(published.exists());
                    }
                    capture.lock().unwrap().push(event);
                }));
                assert!(
                    fvid_media::owned_lossless::supports(&source, &transform, &options),
                    "{codec}"
                );
                let stats =
                    fvid_media::transcode_lossless(&source, &output, transform.clone(), &options)
                        .unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(stats.video_frames, expected.len() as u64);
                assert!(
                    raw_frames(&output) == expected,
                    "{codec}/{operation}/{case}: pixels differ"
                );
                assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
                let mut mux = fvid_media::owned_webm::WebmReader::open(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    Default::default(),
                )
                .unwrap();
                mux.scan_all().unwrap();
                assert_eq!(mux.tracks.len(), selected.len());
                assert_eq!(mux.tags.title, "owned companions");
                assert_eq!(mux.metadata["SOURCE_NOTE"], "public synthetic");
                let mut copied = 0;
                for (mapped, &source_index) in selected.iter().enumerate() {
                    let track = mux.tracks[mapped].clone();
                    assert_eq!(track.name, format!("source-{source_index}"));
                    let uid = mux.track_uids[&track.number];
                    assert_eq!(
                        mux.track_metadata[&uid]["ROLE"],
                        format!("role-{source_index}")
                    );
                    assert_eq!(
                        mux.track_metadata[&uid]["ORIGINAL_NOTE"],
                        ["aac", "video", "opus"][source_index]
                    );
                    if source_index == 1 {
                        assert_eq!(track.codec, "V_FFV1");
                        continue;
                    }
                    assert_eq!(
                        track.codec_private,
                        input.tracks[source_index].codec_private
                    );
                    assert_eq!(
                        track.codec_delay_ns,
                        input.tracks[source_index].codec_delay_ns
                    );
                    let expected_packets = if source_index == 0 { &aac } else { &opus };
                    assert_eq!(packets(&mut mux, track.number), *expected_packets);
                    copied += expected_packets.len() as u64;
                    if source_index == 2 {
                        assert_eq!(opus_pcm(&mut mux, mapped), audio[1]);
                        continue;
                    }
                    let wav = output.with_extension(format!("{mapped}.wav"));
                    fvid_media::decode_audio(
                        &output,
                        &wav,
                        &fvid_control::CopyOptions {
                            streams: vec![mapped],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert_eq!(
                        wave_payload(&wav),
                        audio[if source_index == 0 { 0 } else { 1 }],
                        "{codec}/{operation}/{case}/{source_index}"
                    );
                    std::fs::remove_file(wav).unwrap();
                }
                assert_eq!(stats.copied_packets, copied);
                std::fs::remove_file(output).unwrap();
            }
        }
    }
}
#[test]
fn webm_companion_limits_cancel_and_invalid_selection_do_not_publish() {
    let source = fixture("playback-errors/shared-vp9-companions.mkv");
    for (case, options, error) in [
        (
            0,
            fvid_control::CopyOptions {
                max_packets: Some(1),
                ..Default::default()
            },
            "packet count exceeds limit",
        ),
        (
            1,
            fvid_control::CopyOptions {
                streams: vec![1, 1],
                ..Default::default()
            },
            "invalid or duplicate",
        ),
        (
            2,
            fvid_control::CopyOptions {
                streams: vec![0, 2],
                ..Default::default()
            },
            "selected video stream",
        ),
        (
            3,
            {
                let flag = fvid_control::CancelFlag::new();
                flag.cancel();
                fvid_control::CopyOptions {
                    cancel: Some(flag),
                    ..Default::default()
                }
            },
            "cancelled",
        ),
        (
            4,
            {
                let flag = fvid_control::CancelFlag::new();
                let trigger = flag.clone();
                fvid_control::CopyOptions {
                    cancel: Some(flag),
                    progress: Some(fvid_control::ProgressHook::new(move |event| {
                        assert!(!event.done);
                        if event.packets > 0 {
                            trigger.cancel();
                        }
                    })),
                    ..Default::default()
                }
            },
            "cancelled",
        ),
    ] {
        let output =
            std::env::temp_dir().join(format!("fvid-webm-abort-{}-{case}.mkv", std::process::id()));
        assert!(fvid_media::owned_lossless::supports(
            &source,
            &Default::default(),
            &options
        ));
        let actual = fvid_media::transcode_lossless(&source, &output, Default::default(), &options)
            .unwrap_err();
        assert!(actual.contains(error), "{actual}");
        assert!(!output.exists());
    }
}
