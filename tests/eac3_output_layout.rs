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
    control_with_mode(name,i,width,raw,fvid::codec::ac3_decoder::CompressionMode::Line)
}
fn control_with_mode(name: &str, i: usize, width: u16, raw: bool, mode:fvid::codec::ac3_decoder::CompressionMode) -> Vec<f32> {
    let bytes = read(&format!("{name}-control{i}.ec3"));
    let mut d = if raw {
        Eac3Decoder::new(&[], 48000, width)
    } else {
        Eac3Decoder::for_container_with_compression(
            &[],
            48000,
            width,
            mode,
        )
    }
    .unwrap().with_compression(mode);
    d.decode_complete(&bytes, 0, if name.contains("late") { 162*768 } else { 2304 })
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
    assert_eq!(configuration_speakers(&config(&[(7,0)]),0).unwrap(),Some(six_layout()));
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

fn six_layout() -> SpeakerLayout {
    SpeakerLayout(vec![Speaker::Left,Speaker::Right,Speaker::Centre,Speaker::Lfe,
        Speaker::LeftSurround,Speaker::RightSurround])
}
fn replacement_expected(name:&str,width:usize,raw:bool)->Vec<u8> {
    replacement_expected_mode(name,width,raw,fvid::codec::ac3_decoder::CompressionMode::Line)
}
fn replacement_expected_mode(name:&str,width:usize,raw:bool,mode:fvid::codec::ac3_decoder::CompressionMode)->Vec<u8> {
    let replace=if width<=2 {1} else {2};
    let mut base=control_with_mode(name,0,width as u16,raw,mode);
    let early=control_with_mode(name,1,replace as u16,raw,mode);
    let last=control_with_mode(name,2,replace as u16,raw,mode);
    assert!(early.iter().zip(&last).any(|(a,b)|(a-b).abs()>1e-6));
    assert!(last.iter().any(|s|s.abs()>1e-6));
    for (row,lr) in base.chunks_exact_mut(width).zip(last.chunks_exact(replace)) {
        row[..replace].copy_from_slice(lr);
    }
    base.into_iter().flat_map(f32::to_le_bytes).collect()
}
#[test]
fn native_small_channel_sources_keep_last_dependent_replacement_pcm() {
    for (width,mask) in [(1,4),(2,12),(3,7),(4,15),(5,0x607),(6,0x60f)] {
    for variant in ["plain","transient","gain"] {
        if variant=="gain" && width!=6 {continue;}
        let name=format!("{}-{variant}",if width==6 {"native-six".into()}else{format!("native-{width}")});
        for kind in ["ec3","mkv","mp4","no-dec3"] {
            let mut s=if kind=="no-dec3" {source(&format!("{name}-no-dec3"),"mp4")} else {source(&name,kind)};
            assert_eq!(s.channels(),width as u16);
            if kind=="ec3" {assert_eq!(fvid::codec::eac3_decoder::presentation::packet_speakers(&read(&format!("{name}.ec3")),width as u16).unwrap().wave_mask(),Some(mask));}
            if kind=="no-dec3" {assert!(s.extra_data().is_empty());}
            s.set_eac3_stereo_output(true).unwrap();assert_eq!(s.channels(),2);
            s.set_eac3_stereo_output(false).unwrap();assert_eq!(s.channels(),width as u16);
            s.set_eac3_output_layout(None).unwrap();assert_eq!(s.channels(),width as u16);
            let pcm=replacement_expected(&name,width,kind=="ec3");
            let mut full=Vec::new();let mut sought=Vec::new();let mut offset=0;
            for frames in if kind=="ec3" {[512,768,768]}else{[768,768,768]} {
                full.push((Duration::from_nanos(offset as u64*1_000_000_000/48000),pcm[offset*width*4..(offset+frames)*width*4].to_vec()));
                let start=offset.max(960);
                if start<offset+frames {sought.push((Duration::from_nanos(start as u64*1_000_000_000/48000),pcm[start*width*4..(offset+frames)*width*4].to_vec()));}
                offset+=frames;
            }
            capture::worker_check_with_layout_and_seek_expected(s,&full,Duration::from_millis(20),&sought,mask);
        }
    }
    }
}

#[test]
fn native_six_map_preserves_priming_gain_reset_and_compatibility() {
    use fvid::codec::ac3_decoder::CompressionMode;
    for name in ["native-six-plain","native-six-transient","native-six-gain"] {
        let encoded=read(&format!("{name}.ec3"));
        for raw in [false,true] {
            for mode in [CompressionMode::Line,CompressionMode::Rf] {
                let expected=replacement_expected_mode(name,6,raw,mode);
                let mut decoder=Box::new(if raw {Eac3Decoder::new(&[],48000,6)}else{Eac3Decoder::for_container_with_compression(&[],48000,6,mode)}.unwrap().with_compression(mode));
                decoder.set_output_layout(six_layout()).unwrap();
                for _ in 0..2 {
                    let pcm=decoder.decode_complete(&encoded,0,2304).unwrap().unwrap();
                    assert_eq!(pcm.data,expected,"{name} raw={raw} mode={mode:?}");
                    assert!(decoder.set_output_layout(six_layout()).is_err());
                    decoder.reset();
                }
            }
        }
        let compatible=fvid::playback_ac3::Ac3AudioReader::open_compatible(Cursor::new(encoded.clone()),Default::default()).unwrap();
        let mut decoder=compatible.make_decoder().unwrap();
        let pcm=decoder.decode_complete(&encoded,0,2304).unwrap().unwrap().data;
        // Compatibility intentionally retains the independent signal; the gain
        // witness has a different independent gain from its native common gain.
        if name!="native-six-gain" {
            let independent:Vec<u8>=control(name,0,6,true).into_iter().flat_map(f32::to_le_bytes).collect();
            assert_eq!(pcm,independent);
        }
    }
    let mut d=Box::new(Eac3Decoder::new(&[],48000,6).unwrap());
    d.decode_complete(&read("native-six-plain-control0.ec3"),0,2304).unwrap();
    assert!(d.set_output_layout(six_layout()).is_err());
    let mut stereo=Box::new(Eac3Decoder::new_stereo(&[],48000).unwrap());
    assert!(stereo.set_output_layout(SpeakerLayout(vec![Speaker::Left,Speaker::Right])).is_err());
}


fn top_expected(name:&str,raw:bool)->Vec<u8> {
    let late=name.contains("late");let width=if late {10}else{8};
    let base=control(name,0,6,raw);let dep=control(name,1,2,raw);
    let count=base.len()/6;assert_eq!(dep.len(),count*2);
    let mut out=vec![0f32;count*width];
    for n in 0..count {
        for i in 0..6 {let slot=if late && i>=4 {i+2}else{i};out[n*width+slot]=base[n*6+i];}
        let frame=(n+if raw {256}else{0})/768;
        let slot=if late && frame!=160 {4}else{width-2};
        out[n*width+slot..n*width+slot+2].copy_from_slice(&dep[n*2..n*2+2]);
    }
    out.into_iter().flat_map(f32::to_le_bytes).collect()
}
fn source_packets(s:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)> {
    use fvid::audio::AudioStep;
    let mut d=s.make_decoder().unwrap();let mut out=Vec::new();
    while let Some(step)=s.next_step().unwrap() {
        match step {
            AudioStep::Encoded(p)=>{
                let stamp=s.decoder_stamp(&p);
                if let Some(p)=d.decode_packet(&p.data,stamp,p.duration as u64).unwrap() {
                    if let Some(p)=s.present_decoded(p.packet,p.source_pts).unwrap() {out.push((p.presentation_time(),p.data));}
                }
            },
            AudioStep::ResetDecoder=>s.reset_decoder(&mut d).unwrap(),
            _=>panic!("unexpected source step"),
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p|(p.presentation_time(),p.data)));
    s.validate_eof().unwrap();out
}
#[test]
fn top_surround_roles_missing_from_dec3_keep_native_pcm_clocks_seek_and_rewind() {
    for name in ["top-plain","top-transient","top-late-plain","top-late-transient"] {
        for kind in ["mp4","ec3","mkv"] {
            let mut s=source(name,kind);let late=name.contains("late");let width=if late {10}else{8};
            assert_eq!(s.channels(),width as u16,"{name}.{kind}");
            assert_eq!(s.speaker_mask().unwrap(),None);
            assert!(s.stereo_router().err().unwrap().to_string().contains("extended speaker mask"));
            let pcm=top_expected(name,kind=="ec3");
            let count=if late {162}else{3};let mut offset=0;let mut expected=Vec::new();
            for frame in 0..count {
                let n=if frame==0 && kind=="ec3" {512}else{768};
                expected.push((Duration::from_nanos(offset as u64*1_000_000_000/48000),pcm[offset*width*4..(offset+n)*width*4].to_vec()));offset+=n;
            }
            for _ in 0..2 {assert!(source_packets(&mut *s)==expected,"{name}.{kind}");s.rewind();}
            let target=Duration::from_millis(if late {2562}else{20});let floor=target.as_millis() as usize*48;
            s.seek_time(target);let got=source_packets(&mut *s);
            assert_eq!(got.first().unwrap().0,target);
            assert!(got.into_iter().flat_map(|(_,b)|b).eq(pcm[floor*width*4..].iter().copied()),"seek {name}.{kind}");
        }
    }
}

#[test]
fn mp4_packet_ranges_preserve_cursor_bounds_and_packet_budget() {
    use fvid_media::owned_mp4::{Mp4Reader,Limits};
    let bytes=read("top-late-plain.mp4");
    let mut r=Mp4Reader::open_indexed(Cursor::new(bytes.clone()),Default::default()).unwrap();
    let track=r.tracks().iter().position(|t|t.codec==*b"ec-3").unwrap();
    for sample in [0,160,161] {
        let mut full=Vec::new();r.read_packet(track,sample,&mut full).unwrap();
        for start in [0,1,6,full.len()/2,full.len()] {
            let mut out=[0xa5;17];let n=r.read_packet_range(track,sample,start,&mut out).unwrap();
            assert_eq!(&out[..n],&full[start..(start+17).min(full.len())]);
            assert!(out[n..].iter().all(|&x|x==0xa5));
            let mut again=Vec::new();r.read_packet(track,sample,&mut again).unwrap();assert_eq!(again,full);
        }
        assert!(r.read_packet_range(track,sample,full.len()+1,&mut[0;1]).is_err());
    }
    assert!(r.read_packet_range(usize::MAX,0,0,&mut[0;1]).is_err());
    let mut tiny=Mp4Reader::open_indexed(Cursor::new(bytes),Limits{packet_bytes:1,..Default::default()}).unwrap();
    assert!(tiny.read_packet_range(track,0,0,&mut[0;1]).unwrap_err().to_string().contains("budget"));
}

#[test]
fn top_surround_stereo_policy_keeps_standard_independent_downmix() {
    use fvid::audio_thread::DolbyPlaybackPolicy;
    for name in ["top-plain","top-transient","top-late-plain","top-late-transient"] {
        for kind in ["mp4","ec3","mkv"] {
            let raw=kind=="ec3";let count=if name.contains("late") {162}else{3};
            let bytes=read(&format!("{name}-control0.ec3"));
            let mut control=if raw {Eac3Decoder::new_stereo(&[],48000)}else{Eac3Decoder::for_container_stereo(&[],48000)}.unwrap();
            let pcm=control.decode_complete(&bytes,0,count*768).unwrap().unwrap().data;
            let mut offset=0;let mut expected=Vec::new();let mut sought=Vec::new();
            let target=Duration::from_millis(if count==162 {2562}else{20});let floor=target.as_millis() as usize*48;
            for frame in 0..count {
                let n=if frame==0 && raw {512}else{768};
                expected.push((Duration::from_nanos(offset as u64*1_000_000_000/48000),pcm[offset*8..(offset+n)*8].to_vec()));
                let start=offset.max(floor);
                if start<offset+n {sought.push((Duration::from_nanos(start as u64*1_000_000_000/48000),pcm[start*8..(offset+n)*8].to_vec()));}
                offset+=n;
            }
            capture::worker_check_with_dolby_policy(source(name,kind),&expected,target,&sought,Some(3),
                DolbyPlaybackPolicy{eac3_stereo_output:true,..Default::default()});
        }
    }
}


fn top_layout(late:bool)->SpeakerLayout {
    use Speaker::*;
    let mut roles=vec![Left,Right,Centre,Lfe];
    if late {roles.extend([LeftRear,RightRear]);}
    roles.extend([LeftSurround,RightSurround,TopSurroundLeft,TopSurroundRight]);
    SpeakerLayout(roles)
}
#[test]
fn sources_report_explicit_pcm_roles_without_a_wave_mask() {
    for name in ["top-plain","top-late-transient"] {
        for kind in ["ec3","mp4","mkv"] {
            let mut s=source(name,kind);let expected=top_layout(name.contains("late"));
            assert_eq!(s.speaker_mask().unwrap(),None);
            assert_eq!(s.eac3_output_layout(),Some(expected.clone()),"{name}.{kind}");
            let mut padded=expected.clone();padded.0.push(Speaker::WideLeft);
            s.set_eac3_output_layout(Some(padded.clone())).unwrap();
            assert_eq!(s.eac3_output_layout(),Some(padded.clone()));
            s.set_eac3_stereo_output(true).unwrap();
            assert_eq!(s.eac3_output_layout(),Some(SpeakerLayout(vec![Speaker::Left,Speaker::Right])));
            s.set_eac3_stereo_output(false).unwrap();
            assert_eq!(s.eac3_output_layout(),Some(padded));
            s.set_eac3_output_layout(None).unwrap();
            assert_eq!(s.eac3_output_layout(),Some(expected.clone()));
            s.seek_time(Duration::from_millis(20));
            assert_eq!(s.eac3_output_layout(),Some(expected.clone()));
            s.rewind();assert_eq!(s.eac3_output_layout(),Some(expected));
        }
    }
}


#[test]
fn pcm_role_queries_preserve_c_lfe_and_refuse_unknown_wave_positions() {
    assert_eq!(SpeakerLayout::from_wave_mask(0x60f),Some(six_layout()));
    assert_eq!(SpeakerLayout::from_wave_mask(12),Some(SpeakerLayout(vec![Speaker::Centre,Speaker::Lfe])));
    assert!(SpeakerLayout::from_wave_mask(0).is_none());
    assert!(SpeakerLayout::from_wave_mask(1<<15).is_none());
    assert!(SpeakerLayout::from_wave_mask(u32::MAX).is_none());
    let mut independent=source("native-2-plain-control0","ec3");
    assert_eq!(independent.channels(),2);
    assert_eq!(independent.eac3_output_layout(),Some(SpeakerLayout(vec![Speaker::Centre,Speaker::Lfe])));
    independent.set_eac3_stereo_output(true).unwrap();
    assert_eq!(independent.eac3_output_layout(),Some(SpeakerLayout(vec![Speaker::Left,Speaker::Right])));
    let mut aac=source("aac-factory-stack","mp4");
    assert!(aac.eac3_output_layout().is_none());
    aac.set_eac3_stereo_output(true).unwrap();
    assert!(aac.eac3_output_layout().is_none());
}
#[test]
fn concrete_decoder_reports_configured_and_inferred_output_roles() {
    let bytes=read("top-transient.ec3");let expected=top_layout(false);
    let mut inferred=Box::new(Eac3Decoder::for_container_with_compression(&[],48000,8,fvid::codec::ac3_decoder::CompressionMode::Line).unwrap());
    assert!(inferred.output_layout().is_none());
    assert_eq!(inferred.decode_complete(&bytes,0,2304).unwrap().unwrap().data,top_expected("top-transient",false));
    assert_eq!(inferred.output_layout(),Some(expected.clone()));
    inferred.reset();assert!(inferred.output_layout().is_none());
    inferred.set_output_layout(expected.clone()).unwrap();
    assert_eq!(inferred.output_layout(),Some(expected.clone()));
    inferred.reset();assert_eq!(inferred.output_layout(),Some(expected));
    let stereo=Box::new(Eac3Decoder::new_stereo(&[],48000).unwrap());
    assert_eq!(stereo.output_layout(),Some(SpeakerLayout(vec![Speaker::Left,Speaker::Right])));
}
