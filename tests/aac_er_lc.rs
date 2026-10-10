use std::path::Path;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}

use fvid_media::owned_aac::NativeAacDecoder;
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-er-lc.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn er_lc_native_transitions_tns_and_stereo_match_scalar_pcm() {
    let blob = bytes("aac-er-lc-packets.bin");
    let gold = bytes("aac-er-lc-reference.f32le");
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(
            fvid_media::owned_aac::config::AacConfig::parse(&asc)
                .unwrap()
                .object_type,
            17
        );
        let mut first = vec![];
        for pass in 0..2 {
            if pass != 0 {
                decoder.reset();
            }
            let mut out = vec![];
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let raw = &blob[at..at + len];
                let saved = decoder.checkpoint();
                let mut padded = raw.to_vec();
                padded.push(0); // Valid ER EXT_FILL, rather than trailing garbage.
                let padded_pcm = decoder.decode(&padded).unwrap();
                decoder.restore(&saved).unwrap();
                assert!(decoder.decode(&raw[..raw.len() - 1]).is_err());
                let pcm = decoder.decode(raw).unwrap();
                assert_eq!(padded_pcm, pcm);
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.decode(raw).unwrap(), pcm);
                out.extend(pcm);
            }
            let at = c["reference_offset"].as_u64().unwrap() as usize;
            let count = c["reference_bytes"].as_u64().unwrap() as usize;
            assert_eq!(out.len() * 4, count);
            for (i, (a, b)) in out
                .iter()
                .zip(gold[at..at + count].chunks_exact(4))
                .enumerate()
            {
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 1e-7, "{} sample {i}: {a} vs {b}", c["name"]);
            }
            if pass == 0 {
                first = out;
            } else {
                assert_eq!(out, first);
            }
        }
    }
}
#[test]
fn er_lc_public_mp4_acceptance_matches_owned_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut out = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, owned);
        assert_eq!(out.len(), c["reference_bytes"].as_u64().unwrap() as usize);
        let gold = bytes("aac-er-lc-reference.f32le");
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        for (a, b) in out
            .chunks_exact(4)
            .zip(gold[at..at + out.len()].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
}

use fvid::audio::AudioStream;
fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}

#[test]
fn er_lc_player_ranges_rewind_seek_preserve_pcm_and_channels() {
    use std::{io::Cursor, time::Duration};
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut out = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut out,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            assert_eq!(
                out,
                full[from as usize * 24 * stride..to as usize * 24 * stride]
            );
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(
            (reader.channels(), reader.sample_rate()),
            (c["channels"].as_u64().unwrap() as u16, 24000)
        );
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [1100, 5800, 9000, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), full[landed as usize * stride..]);
        }
    }
}
#[test]
fn er_lc_unimplemented_resilience_and_malformed_videos_refuse_specific_errors() {
    let m = manifest();
    for key in ["rejected", "malformed"] {
        for c in m[key].as_array().unwrap() {
            let mut out = vec![];
            let e = fvid::native_media::decode_mp4_aac_pcm(
                &bytes(c["video"]["file"].as_str().unwrap()),
                &mut out,
            )
            .err()
            .unwrap();
            assert!(e.to_string().contains(c["error"].as_str().unwrap()), "{e}");
        }
    }
}

#[test]
fn er_lc_alignment_bits_do_not_change_pcm() {
    let gold = bytes("aac-er-lc-reference.f32le");
    let m = manifest();
    for c in m["alignment"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["fill_extensions"].as_array().unwrap())
    {
        let mut out = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut out,
        )
        .unwrap();
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let count = c["reference_bytes"].as_u64().unwrap() as usize;
        assert_eq!(out.len(), count);
        for (a, b) in out
            .chunks_exact(4)
            .zip(gold[at..at + count].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
}
