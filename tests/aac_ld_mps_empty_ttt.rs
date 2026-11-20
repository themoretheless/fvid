use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-empty-ttt")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn read(c: &Value, key: &str) -> Vec<u8> {
    std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()
}
fn decode(c: &Value, mkv: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if mkv {
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(c, "file")),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
    } else {
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
    }
    assert_eq!(out.len(), 768 * 6 * 4);
    out
}
#[test]
fn empty_ttt_modes_do_not_change_predictive_pcm() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 112);
    for c in m["cases"].as_array().unwrap() {
        let control = m["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == c["control_name"])
            .unwrap();
        for mkv in [false, true] {
            let expected = decode(control, mkv);
            assert_eq!(decode(c, mkv), expected, "{} mkv={mkv}", c["name"]);
        }
    }
}

fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&packet);
        if let Some(f) = reader
            .decode_source_packet(decoder.as_mut(), &packet, stamp)
            .unwrap()
        {
            if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn downscaled_spatial_playback_seek_and_rewind_keep_source_windows() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(c, "file")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(c, "video")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(play(reader.as_mut()), full);
            for sample in [1, 255, 256, 257, 384, 511, 512, 513, 768] {
                reader.rewind();
                let ticks = (sample as u128 * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * 6
                    * 4;
                assert_eq!(
                    play(reader.as_mut()),
                    full[begin.min(full.len())..],
                    "{} seek{sample} mkv{mkv}",
                    c["name"]
                );
            }
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full);
        }
    }
}

#[test]
fn authored_headers_and_frames_isolate_empty_region_guard() {
    use fvid_media::owned_aac::{aac_ld_mps_config::Config, aac_ld_mps_frame, bits::BitReader};
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let hex = |v: &Value| {
            let s = v.as_str().unwrap();
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>()
        };
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        cfg.validate_low_delay_profile().unwrap();
        assert_eq!(u64::from(cfg.quantization_mode),c["quant"].as_u64().unwrap());
        let inactive = c["inactive_mode"].as_u64().unwrap() as u8;
        if c["empty_region"] == "upper" {
            assert_eq!(cfg.ttt[0].low_mode, 1);
            assert_eq!(cfg.ttt[0].high, Some((inactive, cfg.parameter_bands)));
        } else {
            assert_eq!(cfg.ttt[0].low_mode, inactive);
            assert_eq!(cfg.ttt[0].high, Some((1, 0)));
        }
        let mut active = cfg.clone();
        let (mode, _) = active.ttt[0].high.unwrap();
        active.ttt[0].high = Some((
            mode,
            if c["empty_region"] == "upper" {
                cfg.parameter_bands - 1
            } else {
                1
            },
        ));
        let error = fvid_media::owned_aac::aac_ld_mps_prepare::Decoder::new(&active)
            .unwrap_err()
            .to_string();
        assert!(error.contains("energy quantization for this tree/TTT/arbitrary-tree configuration is not yet qualified"),"{error}");
        let mut d = aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
        for wire in c["reference"].as_array().unwrap() {
            let data = hex(wire);
            let mut b = BitReader::new(&data);
            d.decode(&mut b).unwrap();
            assert!(b.remaining() < 8);
        }
    }
}

fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], c: &Value, i: usize) -> &'a [u8] {
    let r = &c["frames"][i];
    let at = r["offset"].as_u64().unwrap() as usize;
    &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
}

#[test]
fn both_decorrelation_modes_keep_predictive_pcm_and_clocks() {
    use fvid_media::owned_aac::{aac_ld_mps_decor::Mode, AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for mode in [Mode::Real, Mode::Complex] {
        let make = |c: &Value| {
            NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(24000),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        for c in m["cases"].as_array().unwrap() {
            let control = m["controls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["name"] == c["control_name"])
                .unwrap();
            let mut d = make(c);
            let mut want = make(control);
            for i in 0..3 {
                assert_eq!(
                    d.decode_timed_in_clock(packet(&blob, c, i), (i * 256) as i64, 256, 24000)
                        .unwrap(),
                    want.decode_timed_in_clock(
                        packet(&blob, control, i),
                        (i * 256) as i64,
                        256,
                        24000
                    )
                    .unwrap(),
                    "{} frame{i}",
                    c["name"]
                );
            }
            loop {
                let a = d.finish().unwrap();
                let b = want.finish().unwrap();
                assert_eq!(a, b);
                if a.is_none() {
                    break;
                }
            }
        }
    }
}
