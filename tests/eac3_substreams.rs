#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::Eac3Decoder,
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-substreams")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 88);
    c
}
fn open(c: &serde_json::Value, mkv: bool, control: bool) -> Box<dyn AudioStream> {
    let key = match (mkv, control) {
        (false, false) => "file",
        (false, true) => "control",
        (true, false) => "video",
        (true, true) => "control_video",
    };
    let b = Cursor::new(read(c[key].as_str().unwrap()));
    if mkv {
        {
            let mut s=fvid::playback_webm_audio::WebmAudioReader::open(b, Default::default()).unwrap();
            // This helper explicitly tests independent compatibility output.
            s.set_eac3_stereo_output(true).unwrap();
            Box::new(s)
        }
    } else {
        Box::new(fvid::playback_ac3::Ac3AudioReader::open_compatible(b, Default::default()).unwrap())
    }
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        let p = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|d| s.present_decoded(d.packet, d.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = p {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p| (p.presentation_time(), p.data)));
    s.validate_eof().unwrap();
    out
}
#[test]
fn multiplexed_packets_preserve_primary_pcm_without_extra_time_or_rng_changes() {
    for c in cases() {
        let expected = Eac3Decoder::new(&[], 48000, 2)
            .unwrap()
            .decode_complete(&read(c["control"].as_str().unwrap()), 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert!(
            expected
                .chunks_exact(4)
                .any(|x| f32::from_le_bytes(x.try_into().unwrap()).abs() > 1e-6)
        );
        let got = Eac3Decoder::new(&[], 48000, 2)
            .unwrap()
            .decode_complete(&read(c["file"].as_str().unwrap()), 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert_eq!(got.len(), (4608 - 256) * 8);
        assert!(got == expected, "{} primary PCM changed", c["name"]);
    }
}
#[test]
fn skipped_only_packets_emit_nothing_and_do_not_consume_decoder_lead_or_history() {
    for c in cases() {
        let b=read(c["control"].as_str().unwrap());
        let extra=read(c["extra"].as_str().unwrap());
        let mut d=Eac3Decoder::new(&[],48000,2).unwrap();
        for _ in 0..2 {
            let mut reference=Eac3Decoder::new(&[],48000,2).unwrap();
            let mut got=Vec::new();let mut expected=Vec::new();
            for pts in [0,4608] {
                assert!(d.decode_encoded(&extra,pts,0).unwrap().is_none());
                got.extend(d.decode_encoded(&b,pts,4608).unwrap());
                expected.extend(reference.decode_encoded(&b,pts,4608).unwrap());
            }
            while let Some(p)=d.finish_packet().unwrap() {got.push(p.packet);}
            while let Some(p)=reference.finish_packet().unwrap() {expected.push(p.packet);}
            assert_eq!(got.len(),2);
            for (i,(g,e)) in got.iter().zip(&expected).enumerate() {
                assert_eq!(g.pts,[0,4608][i]);
                assert_eq!(g.data,e.data,"{} skipped stream changed running history",c["name"]);
                assert_eq!(g.data.len(),(4608-if i==0 {256}else{0})*8);
            }
            d.reset();
        }
    }
}
#[test]
fn compatible_raw_and_matroska_readers_count_only_primary_samples_and_preserve_rewind() {
    for c in cases() {
        for mkv in [false, true] {
            let mut control = open(&c, mkv, true);
            let expected = packets(&mut *control);
            assert_eq!(expected.len(), 3);
            let mut s = open(&c, mkv, false);
            let got = packets(&mut *s);
            assert_eq!(got, expected, "{} mkv={mkv}", c["name"]);
            assert_eq!(s.duration(), control.duration());
            assert_eq!(s.speaker_mask().unwrap(), Some(3));
            s.rewind();
            assert_eq!(packets(&mut *s), expected);
        }
    }
}
#[test]
fn skipped_stream_lengths_are_bounded_and_reserved_types_are_still_refused() {
    let c = cases().remove(0);
    let extra = read(c["extra"].as_str().unwrap());
    let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
    let error = d
        .decode_encoded(&extra[..extra.len() - 1], 0, 0)
        .err()
        .expect("expected refusal");
    assert!(matches!(error, fvid::Error::Invalid(_)), "{error}");
    let mut reserved = extra.clone();
    reserved[2] |= 0xc0;
    let error = d
        .decode_encoded(&reserved, 0, 0)
        .err()
        .expect("expected refusal");
    assert!(matches!(error, fvid::Error::Unsupported(_)), "{error}");

    let tail = read("truncated-tail.ec3");
    let mut s =
        fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(tail), Default::default()).unwrap();
    let mut count = 0;
    while s.next_step().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(
        count, 3,
        "truncated skipped tail must not erase primary packets"
    );
    assert!(
        s.validate_eof().is_err(),
        "truncated skipped tail must still be reported"
    );
    let mut video = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("truncated-tail.mkv")),
        Default::default(),
    )
    .unwrap();
    for _ in 0..2 {
        assert!(video.next_step().unwrap().is_some());
    }
    let error = video
        .next_step()
        .err()
        .expect("truncated Matroska envelope must fail");
    assert!(
        error
            .to_string()
            .contains("truncated skipped E-AC-3 substream"),
        "{error}"
    );
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn compatible_worker_preserves_multiplexed_primary_pcm_clocks_rewind_and_seek() {
    for c in cases() {
        for mkv in [false, true] {
            let expected = packets(&mut *open(&c, mkv, true));
            let seek = Duration::from_millis(20);
            let mut wanted = expected.clone();
            wanted[0].0 = seek;
            wanted[0].1.drain(..960 * 8);
            capture::worker_check_with_dolby_policy(
                open(&c, mkv, false),
                &expected,
                seek,
                &wanted,
                Some(3),
                fvid::audio_thread::DolbyPlaybackPolicy { eac3_stereo_output:true, ..Default::default() },
            );
        }
    }
}

#[test]
fn native_readers_replace_lr_with_dependent_aht_noise_without_changing_clocks() {
    let native:Vec<_>=cases().into_iter().filter(|c|c["native_control"].is_string()).collect();
    assert_eq!(native.len(),16);
    for c in native {
        for mkv in [false,true] {
            let open_native=|file:&str| -> Box<dyn AudioStream> {
                let b=Cursor::new(read(file));
                if mkv {Box::new(fvid::playback_webm_audio::WebmAudioReader::open(b,Default::default()).unwrap())}
                else {Box::new(fvid::playback_ac3::Ac3AudioReader::open(b,Default::default()).unwrap())}
            };
            let control=if mkv {"native_control_video"}else{"native_control"};
            let input=if mkv {"video"}else{"file"};
            let mut reference=open_native(c[control].as_str().unwrap());
            let expected=packets(&mut *reference);
            let mut s=open_native(c[input].as_str().unwrap());
            assert!(packets(&mut *s)==expected,"{} mkv={mkv}",c["name"]);
            s.rewind();assert!(packets(&mut *s)==expected);
            let seek=Duration::from_millis(20);let mut wanted=expected.clone();
            wanted[0].0=seek;wanted[0].1.drain(..960*8);
            s.rewind();
            capture::worker_check_with_layout_and_seek_expected(s,&expected,seek,&wanted,3);
        }
    }
}
