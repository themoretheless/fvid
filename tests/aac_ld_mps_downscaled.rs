use fvid_media::owned_aac::{
    aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm, bits::BitReader,
};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-downscaled")
}
fn m() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn read(v: &Value) -> Vec<u8> {
    std::fs::read(root().join(v.as_str().unwrap())).unwrap()
}
fn near(a: &[u8], b: &[u8], limit: f32) -> f32 {
    assert_eq!(a.len(), b.len());
    let error = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            (f32::from_le_bytes(x.try_into().unwrap()) - f32::from_le_bytes(y.try_into().unwrap()))
                .abs()
        })
        .fold(0.0f32, f32::max);
    assert!(error < limit, "PCM error {error}");
    error
}
#[test]
fn analytical_short_core_controls_isolate_spatial_admission() {
    let m = m();
    assert_eq!(m["controls"].as_array().unwrap().len(), 42);
    for c in m["controls"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(stats.sample_rate as usize, n(&c["rate"]));
        assert_eq!(stats.sample_frames as usize, n(&c["samples"]));
        near(&pcm, &read(&c["golden"]), 1e-7);
    }
}
fn reference(c: &Value) -> Vec<u8> {
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    cfg.validate_low_delay_profile().unwrap();
    let mut decoder = aac_ld_mps_pcm::Decoder::new(&cfg, Mode::Complex).unwrap();
    let gold = read(&c["core_gold"]);
    let inputs = n(&c["input_channels"]);
    let count = n(&c["group"]) * n(&c["out_n"]);
    let boxes = cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes);
    let order = [0, 1, 2, 3, 6, 7, 4, 5];
    let mut out = Vec::new();
    for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
        let planar = (0..inputs)
            .map(|ch| {
                (0..count)
                    .map(|t| {
                        let at = ((i * count + t) * inputs + ch) * 4;
                        f64::from(f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let pcm = decoder
            .decode(&mut BitReader::new(&hex(wire)), &planar, &vec![None; boxes])
            .unwrap();
        for row in pcm.samples.chunks_exact(n(&c["channels"])) {
            for ch in 0..row.len() {
                out.extend_from_slice(
                    &(row[if matches!(n(&c["tree"]), 4 | 6) {
                        order[ch]
                    } else {
                        ch
                    }] as f32)
                        .to_le_bytes(),
                );
            }
        }
    }
    out
}
#[test]
fn downscaled_spatial_movies_accept_and_match_analytical_core_composition() {
    let m = m();
    assert_eq!(m["cases"].as_array().unwrap().len(), 112);
    let mut values = 0;
    let mut error = 0f32;
    for c in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(stats.sample_rate as usize, n(&c["rate"]));
        assert_eq!(stats.channels as usize, n(&c["channels"]));
        assert_eq!(stats.sample_frames as usize, n(&c["samples"]));
        assert!(pcm.iter().any(|&b| b != 0));
        error = error.max(near(&pcm, &reference(c), 1e-6));
        values += pcm.len() / 4;
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(read(&c["video"])),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, pcm);
        let mut mkv = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(&c["file"])),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mkv, pcm, "{}", c["name"]);
    }
    eprintln!("Downscaled spatial composition: {values} PCM values;max error {error}");
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
    for c in m()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(&c["file"])),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(&c["video"])),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(play(reader.as_mut()), full);
            let core = n(&c["out_n"]);
            let spatial = core * n(&c["group"]);
            for sample in [
                1,
                core - 1,
                core,
                core + 1,
                spatial - 1,
                spatial,
                spatial + 1,
                n(&c["samples"]) / 2,
                n(&c["samples"]),
            ] {
                reader.rewind();
                let ticks = (sample as u128 * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * n(&c["channels"])
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
fn downscaled_native_checkpoints_and_failed_suffix_retry_preserve_core_and_spatial_history() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m()["cases"].as_array().unwrap() {
        let asc = hex(&c["asc"]);
        let rate = n(&c["rate"]) as u32;
        let mut d = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        let mut control = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&row["offset"]);
            let packet = &blob[at..at + n(&row["bytes"])];
            let pts = (i * n(&c["out_n"])) as i64;
            let duration = n(&c["out_n"]) as u64;
            if i == 2 * n(&c["group"]) - 1 {
                let error = d
                    .decode_timed_in_clock(&packet[..packet.len() - 1], pts, duration, rate)
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("spatial child exceeds parent extension"),
                    "{}: {error}",
                    c["name"]
                );
            }
            let before = d.checkpoint();
            let want = control
                .decode_timed_in_clock(packet, pts, duration, rate)
                .unwrap();
            let got = d
                .decode_timed_in_clock(packet, pts, duration, rate)
                .unwrap();
            assert_eq!(got, want, "{} frame{i}", c["name"]);
            if i == 2 * n(&c["group"]) {
                d.restore(&before).unwrap();
                assert_eq!(
                    d.decode_timed_in_clock(packet, pts, duration, rate)
                        .unwrap(),
                    want
                );
            }
        }
        loop {
            let a = d.finish().unwrap();
            let b = control.finish().unwrap();
            assert_eq!(a, b);
            if a.is_none() {
                break;
            }
        }
    }
}
#[test]
fn invalid_short_geometry_and_coupled_sbr_modes_remain_explicit() {
    use fvid_media::owned_aac::aac_ld_mps_native::{Core, Decoder};
    let c = &m()["cases"][0];
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    for factor in [0, 3, 5] {
        let core = Core {
            object_type: 23,
            sample_rate: 48000,
            frame_samples: 512,
            channels: 1,
            has_sbr: false,
            downscale: factor,
        };
        assert!(
            Decoder::new(&cfg, core)
                .unwrap_err()
                .to_string()
                .contains("invalid downscaled AAC LD/ELD spatial core geometry")
        );
    }
    let core = Core {
        object_type: 39,
        sample_rate: 48000,
        frame_samples: 480,
        channels: 1,
        has_sbr: true,
        downscale: 2,
    };
    assert!(
        Decoder::new_with_sbr(&cfg, core, true)
            .unwrap_err()
            .to_string()
            .contains("downscaled coupled LD-SBR")
    );
    // Nominal-rate SSC needs a separate adaptation policy;do not silently retime it.
    let mut nominal = cfg.clone();
    nominal.sample_rate = 48000;
    let core = Core {
        object_type: 23,
        sample_rate: 48000,
        frame_samples: 480,
        channels: 1,
        has_sbr: false,
        downscale: 2,
    };
    assert!(
        Decoder::new(&nominal, core)
            .unwrap_err()
            .to_string()
            .contains("frame geometry disagree")
    );
}
