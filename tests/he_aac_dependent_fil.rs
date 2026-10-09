use fvid_media::owned_aac::{
    aac_native::NativeAacDecoder,
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_history::Stream,
    bits::BitReader,
    config::AudioSpecificConfig,
};
use serde_json::Value;
const RAW: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-dependent-fil-packets.bin");
const BASE: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-dependent-sbr-packets.bin");
const CORE: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-dependent-sbr-core.f32le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-dependent-fil-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let at = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    &RAW[at..at + len]
}
fn reference(c: &Value) -> Vec<f32> {
    let channels = c["channels"].as_u64().unwrap() as usize;
    let slots = c["slots"].as_u64().unwrap() as u8;
    let n = slots as usize * 64;
    let off = c["core_pcm"][0].as_u64().unwrap() as usize;
    let len = c["core_pcm"][1].as_u64().unwrap() as usize;
    let core: Vec<f32> = CORE[off..off + len * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let mut dsp = Dsp::default();
    let mut syntax = Stream::default();
    let mut out = vec![];
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    for (f, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let pcm = &core[f * n * channels..(f + 1) * n * channels];
        let planar: Vec<Vec<f32>> = (0..channels)
            .map(|ch| pcm.chunks_exact(channels).map(|r| r[ch]).collect())
            .collect();
        let refs: Vec<_> = planar.iter().map(Vec::as_slice).collect();
        let raw = hex(row["sbr"].as_str().unwrap());
        let rendered = if raw.is_empty() {
            dsp.process_upsampling(&refs, 48000, slots, mode).unwrap()
        } else {
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            let frame = syntax
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, slots, channels)
                .unwrap();
            dsp.process(&frame, &refs, 48000, slots, mode).unwrap()
        };
        for i in 0..rendered[0].len() {
            for ch in 0..channels {
                out.push(rendered[ch][i] as f32)
            }
        }
    }
    out
}
#[test]
fn dependent_cce_fil_validates_history_without_adding_its_own_sbr_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(config.program.as_ref().unwrap().coupling, vec![(false, 1)]);
        let mut d =
            NativeAacDecoder::new_with_output_rate(&asc, c["output_rate"].as_u64().unwrap() as u32)
                .unwrap();
        let mut actual = vec![];
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            let output = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            actual.extend(output);
        }
        let mut baseline =
            NativeAacDecoder::new_with_output_rate(&asc, c["output_rate"].as_u64().unwrap() as u32)
                .unwrap();
        let baseline_pcm: Vec<f32> = c["baseline_frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| {
                let off = r["offset"].as_u64().unwrap() as usize;
                let len = r["bytes"].as_u64().unwrap() as usize;
                baseline.decode(&BASE[off..off + len]).unwrap()
            })
            .collect();
        assert_eq!(actual, baseline_pcm, "CCE FIL must not add synthesized PCM");
        assert!(
            d.retained_payload_bytes().unwrap() > baseline.retained_payload_bytes().unwrap(),
            "dependent CCE syntax history must be retained separately"
        );
        let saved = d.checkpoint();
        assert!(
            d.retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap()
                > d.retained_payload_bytes().unwrap()
        );
        let expected = reference(c);
        assert_eq!(actual.len(), expected.len());
        assert!(expected.iter().any(|v| v.abs() > 1e-5));
        for (n, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                (a - e).abs() < 2e-7,
                "{} {} {} sample {n}: {a} != {e}",
                c["slots"],
                c["point"],
                c["channels"]
            );
        }
        if c["kind"] == "implicit" {
            let mut detector = NativeAacDecoder::new_with_sbr_detection(&asc).unwrap();
            assert_eq!(detector.sample_rate(), 24000);
            let detected: Vec<f32> = c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| detector.decode(packet(r)).unwrap())
                .collect();
            assert_eq!(detector.sample_rate(), 48000);
            assert_eq!(detected, actual);
            detector.reset();
            assert_eq!(detector.sample_rate(), 24000);
        }
        d.reset();
        let replay: Vec<f32> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| d.decode(packet(r)).unwrap())
            .collect();
        assert_eq!(actual, replay);
    }
}

#[test]
fn dependent_fil_videos_accept_owned_root_pcm_wav_ranges_and_playback() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    use std::io::Cursor;
    let m = manifest();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in m["cases"].as_array().unwrap() {
        if c["video"].is_null() {
            continue;
        }
        let path = root.join(c["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let mut pcm = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, channels as u16, c["samples"].as_u64().unwrap())
        );
        let reader = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let mut rendered = vec![];
        fvid::native_media::decode_mp4_aac_reader(reader, &mut rendered, None).unwrap();
        assert_eq!(pcm, rendered);
        let interval = Some((
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(100),
        ));
        let mut range = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut range,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(range, &pcm[480 * channels * 4..4800 * channels * 4]);
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(&bytes),
                Limits::default(),
            )
            .unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut played = vec![];
            assert_eq!(
                (stream.sample_rate(), stream.channels()),
                (48000, channels as u16)
            );
            while let Some(p) = stream.next_packet().unwrap() {
                let duration = u64::try_from(p.duration).unwrap();
                let frame = decoder
                    .decode_packet(&p.data, p.pts, duration)
                    .unwrap()
                    .unwrap();
                assert_eq!((frame.source_pts, frame.source_duration), (p.pts, duration));
                played.extend(frame.packet.data);
            }
            assert!(decoder.finish_packet().unwrap().is_none());
            assert_eq!(played, pcm);
        }
        let expected = reference(c);
        assert_eq!(pcm.len(), expected.len() * 4);
        for (b, e) in pcm.chunks_exact(4).zip(expected) {
            let a = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - e).abs() < 2e-7, "{}: {a} != {e}", path.display());
        }
        let dest = std::env::temp_dir().join(format!(
            "fvid-dependent-sbr-{}-{}.wav",
            std::process::id(),
            path.file_name().unwrap().to_str().unwrap()
        ));
        fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&dest).unwrap(), None)
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, channels as u16, stats.sample_frames)
        );
        std::fs::remove_file(dest).unwrap();
    }
}

#[test]
fn dependent_cce_crc_and_ps_videos_reject_exact_failure_and_restore_all_sbr_histories() {
    use fvid::container::mp4::Mp4Reader;
    let m = manifest();
    for c in m["invalid"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let mut reader =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for (n, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let mut raw = vec![];
            reader.read_packet(track, n, &mut raw).unwrap();
            assert_eq!(raw, packet(row));
        }
        d.decode(packet(&c["frames"][0])).unwrap();
        let saved = d.checkpoint();
        assert!(
            d.decode(packet(&c["frames"][1]))
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        let after_failure = d.decode(packet(&c["frames"][2])).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(after_failure, d.decode(packet(&c["frames"][2])).unwrap());
    }
}
