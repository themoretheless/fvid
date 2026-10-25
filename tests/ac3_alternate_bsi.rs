#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ac3-alternate-bsi")
}
fn samples(name: &str, ch: u16) -> Vec<f32> {
    let raw = std::fs::read(root().join(name)).unwrap();
    let mut d = fvid::codec::ac3_decoder::Ac3Decoder::new(&[], 48000, ch).unwrap();
    d.decode_complete(&raw, 0, 4608)
        .unwrap()
        .unwrap()
        .data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn alternate_loro_levels_match_independent_scalar_mix_of_unchanged_native_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let centre = [1.414f32, 1.189, 1., 0.841, 0.707, 0.595, 0.5, 0.];
    let surround = [0.841f32, 0.841, 0.841, 0.841, 0.707, 0.595, 0.5, 0.];
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["layout"] == 7)
    {
        let name = c["name"].as_str().unwrap();
        let native = samples(c["control"].as_str().unwrap(), 6);
        assert_eq!(
            samples(&format!("{name}.ac3"), 6),
            native,
            "native payload changed: {name}"
        );
        let stereo = samples(&format!("{name}.ac3"), 2);
        let mono = samples(&format!("{name}.ac3"), 1);
        let mut offset = 0;
        for (i, frames) in [1280, 1536, 1536].into_iter().enumerate() {
            let mix = &c["mixes"][i];
            let (cg, sg) = if c["bsid"] == 6 && !mix.is_null() {
                (
                    centre[mix[3].as_u64().unwrap() as usize],
                    surround[mix[4].as_u64().unwrap() as usize],
                )
            } else {
                (
                    c["basic_centre"].as_f64().unwrap() as f32,
                    c["basic_surround"].as_f64().unwrap() as f32,
                )
            };
            let mut error = 0f32;
            for k in offset..offset + frames {
                let n = &native[k * 6..k * 6 + 6];
                let expected = [
                    (n[0] + cg * n[2] + sg * n[4]).clamp(-1., 1.),
                    (n[1] + cg * n[2] + sg * n[5]).clamp(-1., 1.),
                ];
                for ch in 0..2 {
                    error = error.max((stereo[k * 2 + ch] - expected[ch]).abs());
                }
                let expected_mono =
                    (0.5 * (n[0] + n[1]) + cg * n[2] + 0.5 * sg * (n[4] + n[5])).clamp(-1., 1.);
                error = error.max((mono[k] - expected_mono).abs());
            }
            assert!(error < 2e-7, "{name}: frame{i} downmix error {error}");
            offset += frames;
        }
    }
}
#[test]
fn checked_alternate_metadata_preserves_codes_only_for_bsid6() {
    use fvid::codec::ac3_decoder::checked_alternate_bsi;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let raw =
            std::fs::read(root().join(format!("{}.ac3", c["name"].as_str().unwrap()))).unwrap();
        for (index, p) in raw.chunks_exact(2560).enumerate() {
            if c["bsid"] != 6 {
                assert!(checked_alternate_bsi(p).is_none());
                continue;
            }
            let meta = checked_alternate_bsi(p).unwrap();
            assert_eq!(meta.coding_mode, c["layout"].as_u64().unwrap() as u8);
            assert_eq!(meta.lfe, c["lfe"].as_bool().unwrap());
            let mix = &c["mixes"][index];
            if mix.is_null() {
                assert!(meta.downmix.is_none());
            } else {
                let d = meta.downmix.unwrap();
                assert_eq!(
                    [
                        d.preferred,
                        d.ltrt_centre,
                        d.ltrt_surround,
                        d.loro_centre,
                        d.loro_surround
                    ],
                    std::array::from_fn(|i| mix[i].as_u64().unwrap() as u8)
                );
            }
            let info = &c["info"][index];
            if info.is_null() {
                assert!(meta.information.is_none());
            } else {
                let i = meta.information.unwrap();
                assert_eq!(
                    [
                        i.surround_ex,
                        i.headphone,
                        i.ad_converter as u8,
                        i.reserved,
                        i.encoder as u8
                    ],
                    std::array::from_fn(|j| info[j].as_u64().unwrap() as u8)
                );
            }
            let le: Vec<u8> = p.chunks_exact(2).flat_map(|w| [w[1], w[0]]).collect();
            assert_eq!(checked_alternate_bsi(&le), Some(meta));
            let mut bad = p.to_vec();
            *bad.last_mut().unwrap() ^= 1;
            assert!(checked_alternate_bsi(&bad).is_none());
            for n in [0, 1, 5, 8, 9, 2559] {
                assert!(checked_alternate_bsi(&p[..n]).is_none());
            }
        }
    }
}
#[test]
fn reserved_mix_fields_in_dual_mono_and_stereo_do_not_change_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["layout"] != 7)
    {
        for channels in [1, 2] {
            assert_eq!(
                samples(&format!("{}.ac3", c["name"].as_str().unwrap()), channels),
                samples(c["control"].as_str().unwrap(), channels)
            );
        }
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn alternate_bsi_containers_preserve_native_pcm_video_and_worker_rewind() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as u16;
        let expected = samples(c["control"].as_str().unwrap(), channels);
        for ext in if c["mp4"] == true {
            vec!["mkv", "mp4"]
        } else {
            vec!["mkv"]
        } {
            let bytes = std::fs::read(root().join(format!("{name}.{ext}"))).unwrap();
            let open = || -> Box<dyn AudioStream> {
                if ext == "mkv" {
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
                }
            };
            let mut source = open();
            let full = transitions::collect(source.as_mut());
            let pcm: Vec<f32> = full
                .iter()
                .flat_map(|f| {
                    f.3.chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                })
                .collect();
            assert_eq!(
                &pcm[256 * channels as usize..],
                expected.as_slice(),
                "{name}.{ext}"
            );
            source.rewind();
            transitions::compare(&transitions::collect(source.as_mut()), &full);
            if c["layout"] == 7 && c["mixes"][0][3] == 0 && c["mixes"][0][4] == 0 {
                let expected: Vec<_> = full.into_iter().map(|f| (f.2, f.3)).collect();
                capture::worker_check(open(), &expected);
            }
            let mut video =
                fvid::playback_native::NativeReader::software(Cursor::new(bytes), 16 << 20)
                    .unwrap();
            let mut count = 0;
            while video.read_frame_raw().unwrap().is_some() {
                count += 1;
            }
            assert!(count > 0);
        }
    }
}
