use fvid_media::owned_aac::{NativeAacDecoder, aac_ld_mps_transport::Assembler, bits::BitReader};
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-local-replacement")
}
fn parent() -> PathBuf {
    root().parent().unwrap().join("aac-ld-mps-inband")
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn packet<'a>(blob: &'a [u8], r: &Value) -> &'a [u8] {
    let a = r["offset"].as_u64().unwrap() as usize;
    &blob[a..a + r["bytes"].as_u64().unwrap() as usize]
}
fn play(r: &mut dyn fvid::audio::AudioStream, d: &mut dyn fvid::audio::AudioDecode) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(p) = r.next_packet().unwrap() {
        let stamp = r.decoder_stamp(&p);
        if let Some(f) = r.decode_source_packet(d, &p, stamp).unwrap() {
            if let Some(p) = r.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = d.finish_packet().unwrap() {
        if let Some(p) = r.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn local_fragment_replacements_preserve_messages_pcm_checkpoints_and_navigation() {
    use fvid::audio::AudioStream;
    let m = manifest();
    let data = std::fs::read(root().join("packets.bin")).unwrap();
    let control_data = std::fs::read(parent().join("packets.bin")).unwrap();
    let controls: Value =
        serde_json::from_slice(&std::fs::read(parent().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 120);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(
        cases
            .iter()
            .filter(|c| c["inherited_completes_first"] == true)
            .count(),
        40
    );
    assert_eq!(cases.iter().filter(|c| c["empty"] == true).count(), 40);
    let names: std::collections::BTreeSet<_> =
        cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 120);

    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let control_name = c["control"].as_str().unwrap().trim_end_matches(".mp4");
        let control = controls["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == control_name)
            .unwrap();
        let mut a = Assembler::default();
        let mut b = Assembler::default();
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let samples = c["slots"].as_u64().unwrap() * c["bands"].as_u64().unwrap();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let packet = packet(&data, r);
            let old = packet_from(&control_data, &control["frames"][i]);
            let mut bits = BitReader::new(packet);
            bits.skip(r["prefix_bits"].as_u64().unwrap() as usize)
                .unwrap();
            let messages =
                fvid_media::owned_aac::aac_pce::read_eld_spatial_extensions(&mut bits, &mut a)
                    .unwrap();
            let mut bits = BitReader::new(old);
            bits.skip(control["frames"][i]["prefix_bits"].as_u64().unwrap() as usize)
                .unwrap();
            let expected =
                fvid_media::owned_aac::aac_pce::read_eld_spatial_extensions(&mut bits, &mut b)
                    .unwrap();
            assert_eq!(messages, expected, "{name} AU{i}");
            assert_eq!(a.has_pending(), b.has_pending());
            let checkpoint = d.checkpoint();
            assert!(
                d.decode_timed(
                    &packet[..packet.len() - 2],
                    i as i64 * samples as i64,
                    samples
                )
                .is_err()
            );

            let want = reference
                .decode_timed(old, i as i64 * samples as i64, samples)
                .unwrap();
            let got = d
                .decode_timed(packet, i as i64 * samples as i64, samples)
                .unwrap_or_else(|e| panic!("{name} AU{i}: {e}"));
            assert_eq!(got, want);
            d.restore(&checkpoint).unwrap();
            assert_eq!(
                d.decode_timed(packet, i as i64 * samples as i64, samples)
                    .unwrap(),
                want
            );
        }
        assert!(a.replacements() > 0);
        assert_eq!(b.replacements(), 0);
        loop {
            let f = d.finish().unwrap();
            assert_eq!(f, reference.finish().unwrap());
            if f.is_none() {
                break;
            }
        }
        let bytes = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let old = std::fs::read(parent().join(c["control"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&old, &mut expected).unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut actual).unwrap();
        assert_eq!(actual, expected, "{name}");
        assert_eq!(
            expected.len() as u64,
            c["samples"].as_u64().unwrap() * c["output_channels"].as_u64().unwrap() * 4
        );
        let mkv = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut matroska = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut matroska,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(matroska, expected);
        let readers: [Box<dyn AudioStream>; 2] = [
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(mkv),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for mut r in readers {
            let mut d = r.make_decoder().unwrap();
            assert_eq!(play(r.as_mut(), d.as_mut()), expected);
            r.rewind();
            d.reset();
            assert_eq!(play(r.as_mut(), d.as_mut()), expected);
            for target in [13, samples, c["samples"].as_u64().unwrap(), 7] {
                let at = (u128::from(target) * u128::from(r.timescale())
                    / u128::from(r.sample_rate())) as i64;
                let landed = r.seek_to(at);
                d.reset();
                let first = ((landed.max(0) as u128 * u128::from(r.sample_rate()))
                    .div_ceil(u128::from(r.timescale())) as usize
                    * usize::from(r.channels())
                    * 4)
                .min(expected.len());
                assert_eq!(
                    play(r.as_mut(), d.as_mut()),
                    expected[first..],
                    "{name} seek{target}"
                );
            }
        }
    }
}
fn packet_from<'a>(blob: &'a [u8], r: &Value) -> &'a [u8] {
    packet(blob, r)
}
#[test]
fn inherited_fragment_replacement_remains_explicit_and_correct_retry_keeps_history() {
    let m = manifest();
    assert_eq!(m["errors"].as_array().unwrap().len(), 5);
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let source = std::fs::read(parent().join("packets.bin")).unwrap();
    let controls: Value =
        serde_json::from_slice(&std::fs::read(parent().join("manifest.json")).unwrap()).unwrap();
    for c in m["errors"].as_array().unwrap() {
        let name = c["control"].as_str().unwrap().trim_end_matches(".mp4");
        let control = controls["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap();
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let samples = c["slots"].as_u64().unwrap() * c["bands"].as_u64().unwrap();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let good = packet(&source, &control["frames"][i]);
            if i == 1 {
                let err = d
                    .decode_timed(packet(&blob, r), i as i64 * samples as i64, samples)
                    .unwrap_err()
                    .to_string();
                assert!(
                    err.contains("replaced-fragment source association"),
                    "{err}"
                );
            }
            assert_eq!(
                d.decode_timed(good, i as i64 * samples as i64, samples)
                    .unwrap(),
                reference
                    .decode_timed(good, i as i64 * samples as i64, samples)
                    .unwrap()
            );
        }
        loop {
            let f = d.finish().unwrap();
            assert_eq!(f, reference.finish().unwrap());
            if f.is_none() {
                break;
            }
        }
        let mut pcm = Vec::new();
        let bytes =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let err = fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut pcm)
            .unwrap_err()
            .to_string();
        assert!(err.contains("replaced-fragment source association"));
    }
}
