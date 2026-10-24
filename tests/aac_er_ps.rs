use fvid_media::owned_aac::aac_ps_native::{InBandPsProbe, NativePsAacDecoder};
use serde_json::Value;

fn manifest() -> Value {
    serde_json::from_slice(&bytes("aac-er-ps.json")).unwrap()
}

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}

fn reference_stereo(c: &Value, core_file: &str) -> Vec<f32> {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps::Decoder, bits::BitReader};
    let core: Vec<f32> = bytes(core_file)
        .chunks_exact(4)
        .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
        .collect();
    let n = c["frame_samples"].as_u64().unwrap() as usize;
    let slots = c["slots"].as_u64().unwrap() as u8;
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let mut stage = Decoder::default();
    let mut output = vec![];
    let append = |f: fvid_media::owned_aac::aac_sbr_ps::Frame, out: &mut Vec<f32>| {
        out.extend(
            f.pcm[0]
                .iter()
                .zip(&f.pcm[1])
                .flat_map(|(l, r)| [*l as f32, *r as f32]),
        );
    };
    for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        assert_eq!(kind, 13);
        if let Some(f) = stage
            .read(
                &mut bits,
                raw.len() * 8,
                false,
                &core[i * n..(i + 1) * n],
                48000,
                slots,
                mode,
            )
            .unwrap()
        {
            append(f, &mut output);
        }
    }
    while let Some(f) = stage.finish().unwrap() {
        append(f, &mut output);
    }
    output
}

#[test]
fn er_aac_ps_native_mp4_and_matroska_acceptance() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn play(stream: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = stream.make_decoder().unwrap();
        let mut out = vec![];
        loop {
            use fvid::audio::AudioStep;
            let decoded = match stream.next_step().unwrap() {
                Some(AudioStep::Encoded(p)) => decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap(),
                Some(AudioStep::Pcm(p)) => {
                    out.extend(p.data);
                    continue;
                }
                Some(AudioStep::ReconfigureDecoder) => panic!("unexpected configuration change in constant-setup fixture"),
                Some(AudioStep::ResetDecoder) => {
                    decoder.reset();
                    continue;
                }
                None => {
                    if !stream.drain_decoded_at_eof() {
                        decoder.reset();
                        stream.validate_eof().unwrap();
                        break;
                    }
                    let frame = decoder.finish_packet().unwrap();
                    if frame.is_none() {
                        stream.validate_eof().unwrap();
                        break;
                    }
                    frame
                }
            };
            if let Some(frame) = decoded {
                if let Some(pcm) = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    out.extend(pcm.data);
                }
            }
        }
        out
    }

    let m = manifest();
    let blob = bytes("aac-er-ps-packets.bin");
    let packet = |r: &Value| {
        let at = r["offset"].as_u64().unwrap() as usize;
        &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap() as u32;
        assert!(InBandPsProbe::accepts_mono_program(&asc).unwrap());
        let mut probe = InBandPsProbe::new(&asc, rate).unwrap();
        for r in c["frames"].as_array().unwrap() {
            assert!(probe.read(packet(r)).unwrap());
        }
        assert!(probe.ps_detected());

        let mut d = if c["signal"] == "implicit" {
            NativePsAacDecoder::new_with_in_band_ps(&asc, rate).unwrap()
        } else {
            NativePsAacDecoder::new(&asc).unwrap()
        };
        let mut pcm = vec![];
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = d.checkpoint();
            let err = d.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(
                err.to_string().contains("ER AAC SBR CRC is forbidden"),
                "{}: {err}",
                c["name"]
            );
            let out = d.decode(packet(r)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(out, d.decode(packet(r)).unwrap());
            if let Some(f) = out {
                pcm.extend(f.pcm);
            }
        }
        while let Some(f) = d.finish().unwrap() {
            pcm.extend(f.pcm);
        }

        let gold = reference_stereo(c, c["core_reference"].as_str().unwrap());
        assert_eq!(pcm.len(), gold.len(), "{}", c["name"]);
        for (i, (a, b)) in pcm.iter().zip(&gold).enumerate() {
            assert!((a - b).abs() < 2e-7, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        if let Some(inactive_file) = c["inactive_reference"].as_str() {
            let inactive = reference_stereo(c, inactive_file);
            let delta = gold
                .iter()
                .zip(&inactive)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(delta > 1e-5, "{} LTP not observable: {delta}", c["name"]);
        }

        let mp4_bytes = bytes(c["video"]["file"].as_str().unwrap());
        let mut mp4_pcm = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4_bytes, &mut mp4_pcm).unwrap();
        assert_eq!(mp4_pcm.len(), pcm.len() * 4, "{}", c["name"]);
        assert_eq!(
            mp4_pcm,
            pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),
            "{}",
            c["name"]
        );

        let mut rejected = vec![];
        let bad_mp4 = bytes(c["bad_video"]["file"].as_str().unwrap());
        let err = fvid::native_media::decode_mp4_aac_pcm(&bad_mp4, &mut rejected).unwrap_err();
        assert!(
            err.to_string().contains("ER AAC SBR CRC is forbidden"),
            "{}: {err}",
            c["name"]
        );
        assert!(rejected.is_empty());

        let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(&mp4_bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(play(&mut stream), mp4_pcm);
        stream.rewind();
        assert_eq!(play(&mut stream), mp4_pcm);
        let landed = stream.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut stream), mp4_pcm[landed as usize * 8..]);

        let mka_bytes = bytes(c["matroska"].as_str().unwrap());
        let mut mka_pcm = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mka_bytes, &mut mka_pcm, None)
            .unwrap();
        assert_eq!(mka_pcm, mp4_pcm, "{}", c["name"]);
        let interval = Some((Duration::from_millis(10), Duration::from_millis(100)));
        let mut mka_range = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mka_bytes, &mut mka_range, interval)
            .unwrap();
        assert_eq!(
            mka_range,
            mp4_pcm[(rate as usize / 100) * 8..(rate as usize / 10) * 8]
        );
    }
}
