#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::eac3_decoder::{
        Eac3Decoder,
        presentation::{PresentationDecoder, Speaker, SpeakerLayout},
    },
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-output-layout")
}
fn read(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn layout() -> SpeakerLayout {
    use Speaker::*;
    SpeakerLayout(vec![
        Left,
        Right,
        Centre,
        Lfe,
        LeftRear,
        RightRear,
        LeftSurround,
        RightSurround,
        HeightLeft,
        HeightRight,
    ])
}
fn control(name: &str, i: usize, width: u16, raw: bool) -> Vec<f32> {
    let bytes = read(&format!("{name}-control{i}.ec3"));
    let mut d = if raw {
        Eac3Decoder::new(&[], 48000, width)
    } else {
        Eac3Decoder::for_container_with_compression(
            &[],
            48000,
            width,
            fvid::codec::ac3_decoder::CompressionMode::Line,
        )
    }
    .unwrap();
    d.decode_complete(&bytes, 0, if name == "late" { 162*768 } else { 2304 })
        .unwrap()
        .unwrap()
        .data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn expected(name: &str, raw: bool) -> Vec<u8> {
    let base = control(name, 0, 6, raw);
    let dep = control(name, 1, 2, raw);
    let extra = if name.contains("replace-") {
        Some(control(name, 2, 2, raw))
    } else {
        None
    };
    let count = if name == "late" { 162*768 - if raw {256} else {0} } else if raw { 2048 } else { 2304 };
    assert_eq!(base.len(), count * 6);
    assert_eq!(dep.len(), count * 2);
    assert!(dep.iter().any(|x| x.abs() > 1e-6));
    let mut out = vec![0f32; count * 10];
    for n in 0..count {
        let original = n + if raw { 256 } else { 0 };
        let frame = original / 768;
        let changed = frame == if name == "late" {160} else {1};
        for (input, output) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 6), (5, 7)] {
            out[n * 10 + output] = base[n * 6 + input];
        }
        let pos = if changed && extra.is_none() {
            8
        } else if name.starts_with("six-") && frame != 1 {
            0
        } else {
            4
        };
        out[n * 10 + pos] = dep[n * 2];
        out[n * 10 + pos + 1] = dep[n * 2 + 1];
        if let Some(extra) = &extra {
            let pos = if frame == 1 { 8 } else { 0 };
            out[n * 10 + pos] = extra[n * 2];
            out[n * 10 + pos + 1] = extra[n * 2 + 1];
        }
    }
    out.into_iter().flat_map(f32::to_le_bytes).collect()
}
fn source(name: &str, kind: &str) -> Box<dyn AudioStream> {
    let data = read(&format!("{name}.{kind}"));
    match kind {
        "ec3" => Box::new(
            fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(data), Default::default())
                .unwrap(),
        ),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(data), Default::default())
                .unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn negotiated_output_layout_preserves_changing_roles_history_priming_and_reset() {
    for name in [
        "plain",
        "transient",
        "replace-plain",
        "replace-transient",
        "six-replace-plain",
        "six-replace-transient",
    ] {
        for raw in [false, true] {
            let bytes = read(&format!("{name}.ec3"));
            let want = expected(name, raw);
            let mut d = if raw {
                PresentationDecoder::new(48000, 10)
            } else {
                PresentationDecoder::for_container(48000, 10)
            }
            .unwrap()
            .with_output_layout(layout())
            .unwrap();
            assert_eq!(d.speakers(), Some(&layout()));
            let got = d.decode_complete(&bytes, 0, 2304).unwrap().unwrap().data;
            assert!(
                got == want,
                "{name} raw{raw}: stable output map differs from continuous component controls"
            );
            d.reset();
            assert_eq!(d.speakers(), Some(&layout()));
            assert_eq!(
                d.decode_complete(&bytes, 0, 2304).unwrap().unwrap().data,
                got
            );
            let mut wrapper = if raw {
                Eac3Decoder::new(&[], 48000, 10)
            } else {
                Eac3Decoder::for_container_with_compression(
                    &[],
                    48000,
                    10,
                    fvid::codec::ac3_decoder::CompressionMode::Line,
                )
            }
            .unwrap()
            .with_output_layout(layout())
            .unwrap();
            assert_eq!(
                wrapper
                    .decode_complete(&bytes, 0, 2304)
                    .unwrap()
                    .unwrap()
                    .data,
                got
            );
        }
    }
}
#[test]
fn actual_sources_and_worker_keep_fixed_speakers_at_eof_rewind_and_partial_seek() {
    for name in [
        "plain",
        "transient",
        "replace-plain",
        "replace-transient",
        "six-replace-plain",
        "six-replace-transient",
    ] {
        for kind in ["ec3", "mkv", "mp4"] {
            let s = source(name, kind);
            assert_eq!(s.channels(), 10);
            let duration = s.duration();
            assert_eq!(s.channels(), 10);
            assert_eq!(s.speaker_mask().unwrap(), Some(0x563f));
            assert_eq!(s.duration(), duration);
            let pcm = expected(name, kind == "ec3");
            let mut offset = 0;
            let mut full = Vec::new();
            let mut sought = Vec::new();
            for frames in if kind == "ec3" {
                [512, 768, 768]
            } else {
                [768, 768, 768]
            } {
                full.push((
                    Duration::from_nanos(offset as u64 * 1_000_000_000 / 48000),
                    pcm[offset * 40..(offset + frames) * 40].to_vec(),
                ));
                let start = offset.max(960);
                if start < offset + frames {
                    sought.push((
                        Duration::from_nanos(start as u64 * 1_000_000_000 / 48000),
                        pcm[start * 40..(offset + frames) * 40].to_vec(),
                    ));
                }
                offset += frames;
            }
            capture::worker_check_with_layout_and_seek_expected(
                s,
                &full,
                Duration::from_millis(20),
                &sought,
                0x563f,
            );
        }
    }
}
#[test]
fn unnegotiated_changing_sets_and_invalid_or_incomplete_maps_refuse_explicitly() {
    let raw = read("plain.ec3");
    let mut native = PresentationDecoder::for_container(48000, 8).unwrap();
    let e = native.decode_complete(&raw, 0, 2304).err().unwrap();
    assert!(
        e.to_string()
            .contains("changing E-AC-3 presentation speaker layout")
    );
    let mut l = layout();
    l.0.remove(9);
    l.0.remove(8);
    let mut missing = PresentationDecoder::for_container(48000, 8)
        .unwrap()
        .with_output_layout(l)
        .unwrap();
    let e = missing.decode_complete(&raw, 0, 2304).err().unwrap();
    assert!(e.to_string().contains("outside negotiated output layout"));
    let mut reversed = layout();
    reversed.0.reverse();
    assert!(reversed.validate_output().is_err());
    let mut duplicate = layout();
    duplicate.0[1] = duplicate.0[0];
    assert!(duplicate.validate_output().is_err());
    assert!(
        SpeakerLayout(vec![Speaker::Left; 6])
            .validate_output()
            .is_err()
    );
    assert!(
        PresentationDecoder::new(48000, 8)
            .unwrap()
            .with_output_layout(layout())
            .is_err()
    );
    assert!(
        Eac3Decoder::new_stereo(&[], 48000)
            .unwrap()
            .with_output_layout(layout())
            .is_err()
    );
    let mut s = source("plain", "mkv");
    s.set_eac3_output_layout(Some(layout())).unwrap();
    assert!(s.set_eac3_output_layout(Some(duplicate)).is_err());
    assert_eq!(s.channels(), 10);
    s.set_eac3_stereo_output(true).unwrap();
    assert_eq!(s.channels(), 2);
    s.set_eac3_stereo_output(false).unwrap();
    assert_eq!(s.channels(), 10);
    s.set_eac3_output_layout(None).unwrap();
    assert_eq!(s.channels(), 10);
}

#[test]
fn extended_output_roles_remain_explicit_and_late_map_changes_are_refused() {
    use Speaker::*;
    let extended = SpeakerLayout(vec![
        Left,
        Right,
        Centre,
        Lfe,
        LeftRear,
        RightRear,
        LeftCentre,
        RightCentre,
        BackCentre,
        LeftSurround,
        RightSurround,
        TopCentre,
        HeightLeft,
        HeightCentre,
        HeightRight,
        TopSurroundLeft,
    ]);
    let mut s = source("plain", "mp4");
    s.set_eac3_output_layout(Some(extended)).unwrap();
    assert_eq!(s.channels(), 16);
    assert_eq!(s.speaker_mask().unwrap(), None);
    assert!(
        s.stereo_router()
            .err()
            .unwrap()
            .to_string()
            .contains("extended speaker mask")
    );
    let mut d = s.make_decoder().unwrap();
    let got = d
        .decode_complete(&read("plain.ec3"), 0, 2304)
        .unwrap()
        .unwrap()
        .data;
    let ten = expected("plain", false);
    let slots = [0, 1, 2, 3, 4, 5, 9, 10, 12, 14];
    let mut want = vec![0u8; 2304 * 16 * 4];
    for n in 0..2304 {
        for (i, &j) in slots.iter().enumerate() {
            want[(n * 16 + j) * 4..(n * 16 + j + 1) * 4]
                .copy_from_slice(&ten[(n * 10 + i) * 4..(n * 10 + i + 1) * 4]);
        }
    }
    assert!(got == want);
    s.set_eac3_stereo_output(true).unwrap();
    assert_eq!(s.channels(), 2);
    assert_eq!(s.speaker_mask().unwrap(), Some(3));
    assert!(s.stereo_router().is_ok());
    s.set_eac3_stereo_output(false).unwrap();
    assert_eq!(s.channels(), 16);
    let p = source("plain", "mp4").next_packet().unwrap().unwrap();
    let mut d = PresentationDecoder::for_container(48000, 10)
        .unwrap()
        .with_output_layout(layout())
        .unwrap();
    d.decode_packet(&p.data, 0, 768).unwrap();
    assert!(
        d.with_output_layout(layout())
            .err()
            .unwrap()
            .to_string()
            .contains("precede decoding")
    );
}

#[test]
fn raw_index_records_changing_active_counts_before_output_negotiation() {
    for (name, counts, _first) in [
        ("replace-plain", vec![8, 10, 8], 8),
        ("six-replace-plain", vec![6, 10, 6], 6),
    ] {
        let mut s = fvid::playback_ac3::Ac3AudioReader::open(
            Cursor::new(read(&format!("{name}.ec3"))),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            s.ac3()
                .frames
                .iter()
                .map(|f| f.channels)
                .collect::<Vec<_>>(),
            counts
        );
        assert_eq!(s.channels(), 10);
        s.set_eac3_output_layout(None).unwrap();
        assert_eq!(s.channels(), 10);
        s.set_eac3_output_layout(Some(layout())).unwrap();
        assert_eq!(s.channels(), 10);
    }
}

#[test]
fn non_dolby_reader_factory_fits_a_two_mib_stack() {
    std::thread::Builder::new()
        .name("aac-reader-factory".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut s = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(read("aac-factory-stack.mp4")),
                Default::default(),
            )
            .unwrap();
            assert_eq!((s.sample_rate(), s.channels()), (48000, 2));
            let mut decoder = s.make_decoder().unwrap();
            s.reset_decoder(&mut decoder).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn automatic_map_survives_explicit_override_stereo_and_clear() {
    for kind in ["ec3", "mp4", "mkv"] {
    let mut s = source("six-replace-transient", kind);
    assert_eq!(s.channels(), 10);
    let mut padded = layout();
    padded.0.push(Speaker::WideLeft);
    s.set_eac3_output_layout(Some(padded)).unwrap();
    assert_eq!(s.channels(), 11);
    assert_eq!(s.speaker_mask().unwrap(), None);
    s.set_eac3_output_layout(None).unwrap();
    assert_eq!(s.channels(), 10);
    assert_eq!(s.speaker_mask().unwrap(), Some(0x563f));
    s.set_eac3_stereo_output(true).unwrap();
    assert_eq!(s.channels(), 2);
    s.set_eac3_stereo_output(false).unwrap();
    assert_eq!(s.channels(), 10);
    }
    let compatible = fvid::playback_ac3::Ac3AudioReader::open_compatible(
        Cursor::new(read("six-replace-transient.ec3")), Default::default()).unwrap();
    assert_eq!(compatible.channels(), 6);
}

#[test]
fn dec3_locations_are_lsb_first_selected_and_bounded() {
    use fvid::codec::eac3_decoder::presentation::configuration_speakers;
    fn config(entries: &[(u32,u32)]) -> Vec<u8> {
        let mut bits = Vec::new();
        let mut put = |value: u32, n: u32| { for i in (0..n).rev() { bits.push((value >> i) & 1); } };
        put(0,13); put(entries.len() as u32 - 1,3);
        for &(mode,locations) in entries {
            put(0,2); put(16,5); put(0,5); put(mode,3); put(1,1);
            put(0,3); put(1,4); put(locations,9);
        }
        while bits.len()%8 != 0 { bits.push(0); }
        bits.chunks(8).map(|b|b.iter().fold(0,|v,x|(v<<1)|*x as u8)).collect()
    }
    let data=config(&[(7,2|64),(7,1)]);
    assert_eq!(configuration_speakers(&data,0).unwrap(),Some(layout()));
    let second=configuration_speakers(&data,1).unwrap().unwrap();
    assert_eq!(second.0.len(),8);
    assert!(second.0.contains(&Speaker::LeftCentre));
    assert!(!second.0.contains(&Speaker::HeightLeft));
    assert!(configuration_speakers(&data,2).unwrap().is_none());
    assert!(configuration_speakers(&data[..data.len()-1],0).is_err());
    let extended=configuration_speakers(&config(&[(7,1<<8)]),0).unwrap().unwrap();
    assert!(extended.0.contains(&Speaker::Lfe2));
    assert_eq!(extended.wave_mask(),None);
    assert!(configuration_speakers(&config(&[(7,511)]),0).is_err());
}

#[test]
fn matroska_late_roles_negotiate_before_decode_without_reading_frame_bodies() {
    use std::{sync::{Arc,Mutex},io::{Read,Seek,SeekFrom}};
    struct Logged { data: Cursor<Vec<u8>>, spans: Arc<Mutex<Vec<(u64,usize)>>> }
    impl Read for Logged {
        fn read(&mut self,out:&mut[u8])->std::io::Result<usize> {
            let start=self.data.position(); let n=self.data.read(out)?;
            self.spans.lock().unwrap().push((start,n)); Ok(n)
        }
    }
    impl Seek for Logged {
        fn seek(&mut self,pos:SeekFrom)->std::io::Result<u64> {self.data.seek(pos)}
    }
    let bytes=read("late.mkv");
    let spans=Arc::new(Mutex::new(Vec::new()));
    let s=fvid::playback_webm_audio::WebmAudioReader::open(
        Logged{data:Cursor::new(bytes.clone()),spans:spans.clone()},Default::default()).unwrap();
    assert_eq!(s.channels(),10);
    assert_eq!(s.speaker_mask().unwrap(),Some(0x563f));
    let mut index=fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes),Default::default()).unwrap();
    index.scan_all().unwrap();
    let touched=|offset:u64,size:usize| -> usize {
        spans.lock().unwrap().iter().map(|&(start,n)|
            (start+n as u64).min(offset+size as u64).saturating_sub(start.max(offset)) as usize).sum()
    };
    let mut audio=0;
    let mut video=0;
    for packet in &index.packets {
        if packet.track==1 {
            // The existing initial-cluster open precedes header negotiation.
            if video>0 { assert_eq!(touched(packet.offset,packet.size),0,"video payload read"); }
            video+=1;
        }
        if packet.track==2 {
            if audio>0 { assert!(touched(packet.offset,packet.size)<=32,"read audio frame body at {audio}"); }
            audio+=1;
        }
    }
    assert_eq!(audio,162);
    let pcm=expected("late",false);
    let full:Vec<_>=(0..162).map(|i|(
        Duration::from_millis(i as u64*16),pcm[i*768*40..(i+1)*768*40].to_vec())).collect();
    let start=160*768+96;
    let sought=vec![
        (Duration::from_millis(2562),pcm[start*40..161*768*40].to_vec()),
        (Duration::from_millis(2576),pcm[161*768*40..].to_vec()),
    ];
    capture::worker_check_with_layout_and_seek_expected(
        Box::new(s),&full,Duration::from_millis(2562),&sought,0x563f);
}

#[test]
fn matroska_payload_ranges_restore_prefixes_and_zlib_and_enforce_bounds() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for name in ["matroska-header-stripping/one-byte.mkv",
        "matroska-header-stripping/whole-frame.mkv", "matroska-zlib/identical-zlib.mkv"] {
        let mut r=fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(base.join(name)).unwrap()),Default::default()).unwrap();
        r.scan_all().unwrap();
        let full=r.read_packet(0).unwrap();
        for start in [0,1,full.len()/2,full.len()] {
            let mut out=[0xa5;17];
            let n=r.read_packet_range(0,start,&mut out).unwrap();
            assert_eq!(&out[..n],&full[start..(start+17).min(full.len())]);
            assert!(out[n..].iter().all(|&b|b==0xa5));
        }
        assert!(r.read_packet_range(0,full.len()+1,&mut [0;1]).is_err());
        r.restrict_packet_bytes(full.len()-1);
        assert!(r.read_packet_range(0,0,&mut [0;1]).is_err());
    }
}

#[test]
fn header_only_speaker_probe_handles_word_swapping_without_audio_payloads() {
    use fvid::codec::eac3_decoder::presentation::header_speakers;
    let bytes=read("plain.ec3");
    let mut offset=0; let mut frame=0;
    while offset<bytes.len() {
        let size=(usize::from(bytes[offset+2]&7)*256+usize::from(bytes[offset+3])+1)*2;
        let prefix=&bytes[offset..offset+size.min(16)];
        let roles=header_speakers(prefix).unwrap();
        let expected=if frame%2==0 {
            vec![Speaker::Left,Speaker::Right,Speaker::Centre,Speaker::Lfe,Speaker::LeftSurround,Speaker::RightSurround]
        } else if frame/2==1 {vec![Speaker::HeightLeft,Speaker::HeightRight]}
        else {vec![Speaker::LeftRear,Speaker::RightRear]};
        assert_eq!(roles.0,expected);
        let mut swapped=prefix.to_vec();
        for word in swapped.chunks_exact_mut(2) {word.swap(0,1);}
        assert_eq!(header_speakers(&swapped).unwrap(),roles);
        assert!(header_speakers(&prefix[..5]).is_err());
        offset+=size; frame+=1;
    }
    assert_eq!(frame,6);
}
