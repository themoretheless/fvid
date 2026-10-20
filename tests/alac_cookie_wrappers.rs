#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
#[test]
fn legacy_cookie_wrappers_preserve_pcm_layout_seek_and_exports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-cookie-wrappers");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let depth = c["depth"].as_u64().unwrap();
        let mut expected = Vec::new();
        for n in 0..32 {
            for ch in c["order"].as_array().unwrap() {
                let ch = ch.as_u64().unwrap() as i32;
                let v = ((ch + 1) * 101 + n * 7) * if ch % 2 == 0 { 1 } else { -1 };
                let v = v * c["scale"].as_i64().unwrap() as i32
                    + if c["low_bias"].as_bool().unwrap() {
                        ch + 1
                    } else {
                        0
                    };
                expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
            }
        }
        for extension in ["mkv", "m4a"] {
            let source = root.join(name).with_extension(extension);
            let bytes = std::fs::read(&source).unwrap();
            if extension == "mkv" {
                let mut video = fvid::playback_native::NativeReader::software(
                    Cursor::new(bytes.clone()),
                    32 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    assert!(video.read_frame_raw().unwrap().is_some());
                }
                assert!(video.read_frame_raw().unwrap().is_none());
            }
            for first in [0, 11usize] {
                let mut reader: Box<dyn AudioStream> = if extension == "mkv" {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(
                            Cursor::new(bytes.clone()),
                            Default::default(),
                        )
                        .unwrap_or_else(|e| panic!("{name}: {e}")),
                    )
                } else {
                    Box::new(
                        fvid::playback_mp4_audio::Mp4AudioReader::open(
                            Cursor::new(bytes.clone()),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                };
                assert_eq!(
                    reader.speaker_mask().unwrap(),
                    Some(c["mask"].as_u64().unwrap() as u32)
                );
                reader.seek_to(if extension == "mkv" && first != 0 {
                    208334
                } else {
                    first as i64
                });
                let mut decoder = reader.make_decoder().unwrap();
                let mut actual = Vec::new();
                while let Some(p) = reader.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.extend(out.data);
                    }
                }
                assert_eq!(
                    actual,
                    expected[first * channels * 4..],
                    "{name}/{extension}/{first}"
                );
            }
            let output = std::env::temp_dir().join(format!(
                "fvid-alac-cookie-{}-{name}-{extension}.wav",
                std::process::id()
            ));
            struct Remove(std::path::PathBuf);
            impl Drop for Remove {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let _cleanup = Remove(output.clone());
            fvid_media::decode_audio(&source, &output, &Default::default())
                .unwrap_or_else(|e| panic!("{name}/{extension}: {e}"));
            let mut file = std::fs::File::open(&output).unwrap();
            let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(wave.float);
            assert_eq!(wave.channel_mask, c["mask"].as_u64().unwrap() as u32);
            file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
            let mut actual = vec![0; wave.data_bytes as usize];
            file.read_exact(&mut actual).unwrap();
            assert_eq!(actual, expected, "{name}/{extension} export");
            let native_output = output.with_extension("f32le");
            let _native_cleanup = Remove(native_output.clone());
            fvid::native_export::export_audio_pcm_selected(
                &source,
                &native_output,
                None,
                1.0,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_else(|e| panic!("{name}/{extension} native export: {e}"));
            assert_eq!(
                std::fs::read(native_output).unwrap(),
                expected,
                "{name}/{extension} native PCM export"
            );
        }
    }
}
#[test]
fn malformed_legacy_cookie_headers_refuse_the_specific_atom_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-cookie-wrappers");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in m["invalid"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        let e = match fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        ) {
            Ok(_) => panic!("{}: invalid setup accepted", c["file"]),
            Err(e) => e.to_string(),
        };
        assert!(
            e.contains(c["error"].as_str().unwrap()),
            "{}: {e}",
            c["file"]
        );
    }
}

#[test]
fn truncated_cookie_prefixes_are_checked_consistently_by_all_entry_points() {
    use fvid::codec::alac_decoder::AlacDecoder as Frontend;
    use fvid_media::owned_alac::AlacDecoder as Library;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-cookie-wrappers");
    for style in [
        "atom",
        "atom-terminated",
        "format-atom",
        "format-fixed-terminated",
        "atom-fixed",
    ] {
        let bytes = std::fs::read(root.join(format!("2-16-discrete-{style}.m4a"))).unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        let cookie = reader.extra_data().to_vec();
        let packet = reader.next_packet().unwrap().unwrap().data;
        for length in 0..=cookie.len() {
            let prefix = &cookie[..length];
            let accepted = Frontend::new(prefix, 48000, 2).is_ok();
            assert_eq!(Library::new(prefix, 48000, 2).is_ok(), accepted);
            assert_eq!(
                Frontend::packet_samples(prefix, 48000, 2, &packet).is_ok(),
                accepted
            );
            assert_eq!(
                Library::packet_samples(prefix, 48000, 2, &packet).is_ok(),
                accepted
            );
            assert_eq!(
                Frontend::configuration_mask(prefix, 48000, 2).is_ok(),
                accepted
            );
            assert_eq!(
                Library::configuration_mask(prefix, 48000, 2).is_ok(),
                accepted
            );
            assert_eq!(
                Frontend::decode_admission_bytes(prefix, 48000, 2).is_ok(),
                accepted
            );
            assert_eq!(
                Library::decode_admission_bytes(prefix, 48000, 2).is_ok(),
                accepted
            );
            // A prefix may legitimately end at the complete cookie before an
            // optional terminator. If accepted, it must decode the same PCM.
            if accepted {
                let mut decoder = Frontend::new(prefix, 48000, 2).unwrap();
                let pcm = decoder.decode_pcm(&packet).unwrap();
                assert_eq!(pcm.len(), 32);
                assert_eq!(pcm[0], 101. / 32768.);
            }
        }
    }
}
