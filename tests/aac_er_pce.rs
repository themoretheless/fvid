use fvid_media::owned_aac::{
    aac_ps_native::NativePsAacDecoder,
    config::{AacConfig, AudioSpecificConfig},
    NativeAacDecoder,
};
use serde_json::Value;
use std::{io::Cursor, path::Path};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}

fn manifest() -> Value {
    serde_json::from_slice(&bytes("aac-er-pce.json")).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}

#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
    loop {
        use fvid::audio::AudioStep;
        let decoded = match reader.next_step().unwrap() {
            Some(AudioStep::Encoded(p)) => decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap(),
            Some(AudioStep::Pcm(p)) => {
                output.extend(p.data);
                continue;
            }
            Some(AudioStep::ReconfigureDecoder) => panic!("unexpected configuration change in constant-setup fixture"),
            Some(AudioStep::ResetDecoder) => {
                decoder.reset();
                continue;
            }
            None => {
                if !reader.drain_decoded_at_eof() {
                    decoder.reset();
                    reader.validate_eof().unwrap();
                    break;
                }
                let frame = decoder.finish_packet().unwrap();
                let Some(frame) = frame else {
                    reader.validate_eof().unwrap();
                    break;
                };
                Some(frame)
            }
        };
        if let Some(frame) = decoded {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    output
}

#[test]
fn er_lc_er_ltp_and_er_ld_pce_native_mp4_and_matroska_acceptance() {
    let m = manifest();
    let blob = bytes("aac-er-pce-packets.bin");
    let gold = bytes("aac-er-pce-reference.f32le");

    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let kind = c["kind"].as_str().unwrap();
        let aot = c["aot"].as_u64().unwrap() as u32;
        let asc = hex(c["asc"].as_str().unwrap());
        let parsed = AudioSpecificConfig::parse(&asc).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(parsed.core.object_type, aot);
        assert_eq!(parsed.core.channel_configuration, 0);
        let program = parsed.program.as_ref().expect("PCE must be parsed");
        assert_eq!(program.sample_rate, c["rate"].as_u64().unwrap() as u32);
        let serialized = program
            .audio_specific_config()
            .unwrap_or_else(|e| panic!("{name} serialize: {e}"));
        let reparsed = AacConfig::parse_with_program(&serialized).unwrap();
        assert_eq!(reparsed.0.object_type, aot);
        assert_eq!(reparsed.1.as_ref(), Some(program));

        if kind == "ps" {
            let control_asc = hex(c["control_asc"].as_str().unwrap());
            let mut decoder = NativePsAacDecoder::new(&asc).unwrap();
            let mut control = NativePsAacDecoder::new(&control_asc).unwrap();
            let mut actual = Vec::new();
            let mut expected = Vec::new();
            for (row, crow) in c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(c["control_frames"].as_array().unwrap())
            {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let cat = crow["offset"].as_u64().unwrap() as usize;
                let clen = crow["bytes"].as_u64().unwrap() as usize;
                let saved = decoder.checkpoint();
                let mut wrong = blob[at..at + len].to_vec();
                wrong[0] ^= 0x10; // Flip PCE element tag
                assert!(
                    decoder
                        .decode(&wrong)
                        .unwrap_err()
                        .to_string()
                        .contains("PS AAC mono tag is not configured by PCE")
                );
                let out = decoder.decode(&blob[at..at + len]).unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(out, decoder.decode(&blob[at..at + len]).unwrap());
                if let Some(frame) = out {
                    actual.extend(frame.pcm);
                }
                if let Some(frame) = control.decode(&blob[cat..cat + clen]).unwrap() {
                    expected.extend(frame.pcm);
                }
            }
            if let Some(frame) = decoder.finish().unwrap() {
                actual.extend(frame.pcm);
            }
            if let Some(frame) = control.finish().unwrap() {
                expected.extend(frame.pcm);
            }
            assert_eq!(actual, expected, "{name}");

            let mp4 = bytes(c["video"]["file"].as_str().unwrap());
            let mut mp4_pcm = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut mp4_pcm).unwrap();
            let expected_bytes: Vec<u8> = expected.iter().flat_map(|v| v.to_le_bytes()).collect();
            assert_eq!(mp4_pcm, expected_bytes, "{name} MP4");

            let mkv = bytes(c["mkv"].as_str().unwrap());
            #[cfg(feature = "player")]
            {
                let mut mp4_reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(&mp4),
                    Default::default(),
                )
                .unwrap();
                assert_eq!(play(&mut mp4_reader), expected_bytes, "{name} MP4 player");
                let mut mkv_reader = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(&mkv),
                    Default::default(),
                )
                .unwrap();
                assert_eq!(play(&mut mkv_reader), expected_bytes, "{name} MKV player");
            }
            continue;
        }

        let channels = c["channels"].as_u64().unwrap() as usize;
        let n = c["n"].as_u64().unwrap() as usize;
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(decoder.channel_mask(), c["mask"].as_u64().unwrap() as u32);
        let retained = decoder.retained_payload_bytes().unwrap();
        let mut all = Vec::new();
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + len];
            let saved = decoder.checkpoint();
            let mut wrong = raw.to_vec();
            wrong[0] ^= 0x80; // Flip first element's 4-bit tag so it is absent from PCE
            let err = decoder.decode(&wrong).unwrap_err();
            assert!(
                err.to_string().contains("AAC element is absent from PCE"),
                "{name}: {err}"
            );
            let pts = (index * n) as i64;
            let frame = decoder
                .decode_timed(raw, pts, n as u64)
                .unwrap()
                .unwrap();
            assert_eq!((frame.pts, frame.duration), (pts, n as u64));
            decoder.restore(&saved).unwrap();
            assert_eq!(frame.samples, decoder.decode(raw).unwrap());
            all.extend(frame.samples);
            assert_eq!(decoder.retained_payload_bytes().unwrap(), retained);
        }

        let ref_start = c["reference_offset"].as_u64().unwrap() as usize;
        let ref_bytes = c["reference_bytes"].as_u64().unwrap() as usize;
        assert_eq!(all.len() * 4, ref_bytes, "{name}");
        for (i, (actual, raw)) in all
            .iter()
            .zip(gold[ref_start..ref_start + ref_bytes].chunks_exact(4))
            .enumerate()
        {
            let expected = f32::from_le_bytes(raw.try_into().unwrap());
            assert!(
                (*actual - expected).abs() < 2e-7,
                "{name} sample {i}: {actual} vs {expected}"
            );
        }

        let mp4 = bytes(c["video"]["file"].as_str().unwrap());
        let mut public = Vec::new();
        let mut owned = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut public).unwrap();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(public, owned, "{name}");
        let expected_bytes: Vec<u8> = all.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(public, expected_bytes, "{name} MP4 bytes");

        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let stride = channels * 4;
            let mut mp4_reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(&mp4),
                Default::default(),
            )
            .unwrap();
            assert_eq!(play(&mut mp4_reader), expected_bytes, "{name} MP4 play");
            mp4_reader.rewind();
            assert_eq!(play(&mut mp4_reader), expected_bytes, "{name} MP4 rewind");
            let landed = mp4_reader.seek_to((2 * n) as i64);
            assert_eq!(
                play(&mut mp4_reader),
                expected_bytes[landed as usize * stride..],
                "{name} MP4 seek"
            );

            let mkv = bytes(c["mkv"].as_str().unwrap());
            let mut mkv_reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(&mkv),
                Default::default(),
            )
            .unwrap();
            assert_eq!(play(&mut mkv_reader), expected_bytes, "{name} MKV play");
        }
    }
}
