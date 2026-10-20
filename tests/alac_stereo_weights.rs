#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
fn bits(data: &[u8], start: usize) -> u8 {
    (start..start + 8).fold(0, |v, i| (v << 1) | ((data[i / 8] >> (7 - i % 8)) & 1))
}
#[test]
fn signed_stereo_weights_preserve_literal_channels_low_bits_and_seek() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-stereo-weights");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let depth = c["depth"].as_u64().unwrap();
        let extra = c["extra"].as_u64().unwrap();
        let mut expected = Vec::new();
        for n in 0..32 {
            let sign = if n % 2 == 0 { 1 } else { -1 };
            for (ch, v) in [sign * (7 + (n % 8) * 3), -sign * (11 + (n % 5) * 2)]
                .into_iter()
                .enumerate()
            {
                let v = (v << extra)
                    + if extra == 0 {
                        0
                    } else if extra == 8 {
                        [0x12, 0xab][ch]
                    } else {
                        [0x1234, 0xabcd][ch]
                    };
                expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
            }
        }
        let cookie = std::fs::read(root.join(format!("{name}.cookie"))).unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        assert_eq!(bits(&packet, 55), c["shift"].as_u64().unwrap() as u8);
        assert_eq!(bits(&packet, 63), c["weight"].as_i64().unwrap() as u8);
        let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
        let actual: Vec<_> = decoder
            .decode_pcm(&packet)
            .unwrap()
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(actual.len(), 128);
        assert_eq!(
            actual
                .iter()
                .zip(&expected[..128])
                .position(|(a, b)| a != b),
            None,
            "{name}: first wrong PCM byte"
        );
        for extension in ["mkv", "m4a"] {
            let source = root.join(format!("{name}.{extension}"));
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
                        .unwrap(),
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
                reader.seek_to(if extension == "mkv" && first != 0 {
                    208334
                } else {
                    first as i64
                });
                let mut decoder = reader.make_decoder().unwrap();
                let mut pcm = Vec::new();
                while let Some(p) = reader.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        pcm.extend(out.data);
                    }
                }
                assert_eq!(pcm, expected[first * 8..], "{name}/{extension}/{first}");
            }
            let output = std::env::temp_dir().join(format!(
                "fvid-alac-stereo-{}-{name}-{extension}.wav",
                std::process::id()
            ));
            struct Remove(std::path::PathBuf);
            impl Drop for Remove {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let _cleanup = Remove(output.clone());
            fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
            let mut file = std::fs::File::open(&output).unwrap();
            let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(wave.float);
            assert_eq!(wave.channel_mask, 3);
            file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
            let mut pcm = vec![0; wave.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            assert_eq!(pcm, expected, "{name}/{extension} WAVE export");
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
            .unwrap();
            assert_eq!(
                std::fs::read(native_output).unwrap(),
                expected,
                "{name}/{extension} native PCM export"
            );
        }
    }
}

#[test]
fn only_zero_weight_may_ignore_an_oversized_mix_shift() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-stereo-weights");
    let cookie = std::fs::read(root.join("16-low0-weight-128-shift0.cookie")).unwrap();
    let direct = std::fs::read(root.join("16-low0-weight0-shift255.packet")).unwrap();
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    assert_eq!(decoder.decode_pcm(&direct).unwrap().len(), 32);
    let bytes = std::fs::read(root.join("invalid-negative-shift.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes), 32 << 20).unwrap();
    for _ in 0..2 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    let bad = std::fs::read(root.join("invalid-negative-shift.packet")).unwrap();
    assert_eq!(bits(&bad, 55), 255);
    assert_eq!(bits(&bad, 63), 128);
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    assert!(
        decoder
            .decode_pcm(&bad)
            .unwrap_err()
            .to_string()
            .contains("ALAC stereo unmix shifts a sample by more than it holds")
    );
    let mut decoder = fvid_media::owned_alac::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    assert!(
        decoder
            .decode_pcm(&bad)
            .unwrap_err()
            .to_string()
            .contains("ALAC stereo unmix shifts a sample by more than it holds")
    );
}
