use serde_json::Value;
use std::io::Cursor;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-gain-control.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn bytes(video: &Value) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(video["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn empty_gain_control_videos_preserve_independent_core_and_qualified_ps_pcm() {
    let gold = include_bytes!("fixtures/playback-errors/aac-gain-control-core.f32le");
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(&c["video"]);
        let baseline = bytes(&c["baseline"]);
        let mut actual = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut expected = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&baseline),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(actual, expected, "{}", c["video"]["file"]);
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut root).unwrap();
        assert_eq!(root, actual);
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (
                c["output_rate"].as_u64().unwrap() as u32,
                c["channels"].as_u64().unwrap() as u16,
                c["samples"].as_u64().unwrap()
            )
        );
        if c["kind"] == "LC" {
            let off = c["core_offset"].as_u64().unwrap() as usize;
            for (a, e) in actual
                .chunks_exact(4)
                .zip(gold[off..off + actual.len()].chunks_exact(4))
            {
                assert!(
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(e.try_into().unwrap()))
                    .abs()
                        < 2e-7
                );
            }
        }
    }
}
#[test]
fn active_gain_video_refuses_exact_tool_and_rolls_back_overlap() {
    use fvid::container::mp4::Mp4Reader;
    use fvid_media::owned_aac::aac_native::NativeAacDecoder;
    for c in manifest()["invalid"].as_array().unwrap() {
        let mut r = Mp4Reader::open(Cursor::new(bytes(&c["video"])), Default::default()).unwrap();
        let track = r
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut packets = vec![];
        let raw = include_bytes!("fixtures/playback-errors/aac-gain-control-packets.bin");
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let mut packet = vec![];
            r.read_packet(track, i, &mut packet).unwrap();
            let off = row["offset"].as_u64().unwrap() as usize;
            assert_eq!(
                packet,
                &raw[off..off + row["bytes"].as_u64().unwrap() as usize]
            );
            packets.push(packet);
        }
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        d.decode(&packets[0]).unwrap();
        let saved = d.checkpoint();
        assert!(
            d.decode(&packets[1])
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        let after = d.decode(&packets[2]).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(after, d.decode(&packets[2]).unwrap());
    }
}

#[test]
fn gain_control_syntax_covers_all_windows_and_rolls_back_every_truncation() {
    use fvid_media::owned_aac::{
        aac_gain_control::GainControl, aac_synthesis::WindowSequence, bits::BitReader,
    };
    fn field(v: usize, n: usize) -> String {
        format!("{v:0n$b}")
    }
    fn packed(bits: &str) -> Vec<u8> {
        let mut b = vec![0; bits.len().div_ceil(8)];
        for (i, v) in bits.bytes().enumerate() {
            if v == b'1' {
                b[i / 8] |= 1 << (7 - i % 8);
            }
        }
        b
    }
    for (sequence, windows, transition, width) in [
        (WindowSequence::OnlyLong, 1, false, 5),
        (WindowSequence::LongStart, 2, true, 2),
        (WindowSequence::EightShort, 8, false, 2),
        (WindowSequence::LongStop, 2, true, 5),
    ] {
        for bands in 0..=3 {
            for adjustments in [0, 7] {
                let mut bits = field(bands, 2);
                for _ in 0..bands {
                    for w in 0..windows {
                        bits += &field(adjustments, 3);
                        let n = if transition && w == 0 { 4 } else { width };
                        for i in 0..adjustments {
                            bits += &(field(i + 1, 4) + &field((1 << n) - 1, n));
                        }
                    }
                }
                // Place the syntax at an unaligned offset ending exactly at EOF.
                let prefix = (8 - bits.len() % 8) % 8;
                let wire = packed(&("0".repeat(prefix) + &bits));
                let mut reader = BitReader::new(&wire);
                reader.skip(prefix).unwrap();
                let data = GainControl::read(&mut reader, sequence).unwrap();
                assert_eq!(data.bands.len(), bands);
                assert_eq!(data.is_empty(), bands == 0 || adjustments == 0);
                assert_eq!(reader.remaining(), 0);
                for rows in &data.bands {
                    assert_eq!(rows.len(), windows);
                    for (w, row) in rows.iter().enumerate() {
                        assert_eq!(row.len(), adjustments);
                        let n = if transition && w == 0 { 4 } else { width };
                        for (i, a) in row.iter().enumerate() {
                            assert_eq!(a.level, i as u8 + 1);
                            assert_eq!(a.location, (1 << n) - 1);
                        }
                    }
                }
                for end in 0..wire.len() {
                    let mut short = BitReader::new(&wire[..end]);
                    if short.skip(prefix).is_err() {
                        continue;
                    }
                    let position = short.position();
                    assert!(GainControl::read(&mut short, sequence).is_err());
                    assert_eq!(short.position(), position);
                }
            }
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn empty_gain_videos_keep_playback_seek_and_repeated_ranges() {
    use fvid::audio::AudioStream;
    fn play(s: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut d = s.make_decoder().unwrap();
        let mut out = vec![];
        while let Some(p) = s.next_packet().unwrap() {
            if let Some(f) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(pcm) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                    out.extend(pcm.data);
                }
            }
        }
        if let Some(f) = d.finish_packet().unwrap() {
            if let Some(pcm) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(pcm.data);
            }
        }
        out
    }
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(&c["video"]);
        let mut pcm = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut s =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut s), pcm);
        s.rewind();
        assert_eq!(play(&mut s), pcm);
        let landed = s.seek_to(2400);
        let size = c["channels"].as_u64().unwrap() as usize * 4;
        assert_eq!(play(&mut s), pcm[landed as usize * size..]);
        let rate = c["output_rate"].as_u64().unwrap() as usize;
        for _ in 0..2 {
            let mut range = vec![];
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut range,
                Some((
                    std::time::Duration::from_millis(10),
                    std::time::Duration::from_millis(100),
                )),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(range, pcm[rate / 100 * size..rate / 10 * size]);
        }
    }
}
