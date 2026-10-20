#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn alac_long_frames_and_short_packets_with_large_cookie_limits_play_and_seek() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-long-frames");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let count = case["samples"].as_u64().unwrap() as usize;
        let expected: Vec<u8> = (0..count * 2)
            .flat_map(|n| {
                let n = n % count;
                [101 + (n % 31) as i32 * 7, -202 - (n % 31) as i32 * 7]
            })
            .flat_map(|x| (x as f32 / 32768.).to_le_bytes())
            .collect();
        for extension in ["mkv", "m4a"] {
            let bytes = std::fs::read(root.join(format!("{name}.{extension}"))).unwrap();
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
            for first in [0, 1, count + 1] {
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
                let target = if extension == "mkv" {
                    (first as u64 * 1_000_000_000 / 48000) as i64
                } else {
                    first as i64
                };
                reader.seek_to(target);
                let mut decoder = reader
                    .make_decoder()
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                let mut actual = Vec::new();
                while let Some(p) = reader.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap_or_else(|e| panic!("{name}: {e}"))
                        .unwrap();
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.extend(out.data);
                    }
                }
                assert_eq!(actual, expected[first * 8..], "{name}/{extension}/{first}");
            }

            use std::io::{Read, Seek, SeekFrom};
            let output = std::env::temp_dir().join(format!(
                "fvid-alac-long-{}-{name}-{extension}.wav",
                std::process::id()
            ));
            struct Remove(std::path::PathBuf);
            impl Drop for Remove {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let _cleanup = Remove(output.clone());
            fvid_media::decode_audio(
                &root.join(format!("{name}.{extension}")),
                &output,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}/{extension}: {e}"));
            let mut file = std::fs::File::open(output).unwrap();
            let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(wave.float);
            assert_eq!(wave.channels, 2);
            file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
            let mut pcm = vec![0; wave.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            assert_eq!(pcm, expected, "{name}/{extension} WAVE");
        }
    }
}

#[test]
fn enormous_declared_raw_count_refuses_before_sample_allocation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-long-frames");
    let cookie = std::fs::read(root.join("maximum.cookie")).unwrap();
    let packet = std::fs::read(root.join("truncated-raw.packet")).unwrap();
    let bytes = std::fs::read(root.join("truncated-raw.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    assert!(
        decoder
            .decode_pcm(&packet)
            .unwrap_err()
            .to_string()
            .contains("ALAC frame is short of its samples")
    );
    let mut decoder = fvid_media::owned_alac::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    assert!(
        decoder
            .decode_pcm(&packet)
            .unwrap_err()
            .to_string()
            .contains("ALAC frame is short of its samples")
    );
}
