use std::{io::Cursor,path::PathBuf};
fn root()->PathBuf {PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")}
fn open(path:&std::path::Path)->fvid_media::owned_webm::WebmReader<Cursor<Vec<u8>>>{
    let mut r=fvid_media::owned_webm::WebmReader::open(Cursor::new(std::fs::read(path).unwrap()),Default::default()).unwrap();r.scan_all().unwrap();r
}
#[test]
fn hevc_timed_untimed_and_companion_exports_preserve_video_and_audio_clocks(){
    let m:serde_json::Value=serde_json::from_slice(&std::fs::read(root().join("hevc-matroska-timeline.json")).unwrap()).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(),256);
    for (index,c) in m["cases"].as_array().unwrap().iter().enumerate(){
        let source=root().join(c["file"].as_str().unwrap());
        let output=std::env::temp_dir().join(format!("fvid-hevc-clock-{}-{index}.mkv",std::process::id()));
        let mut input=open(&source);
        let mut audio=Vec::new();
        for i in 0..input.packets.len(){let p=input.packets[i].clone();if p.track!=1{
            let track=input.tracks.iter().find(|t|t.number==p.track).unwrap().clone();let packet=input.read_packet(i).unwrap();
            let duration=p.duration_ns.or((track.default_duration_ns!=0).then_some(track.default_duration_ns)).unwrap_or_else(||{
                if track.codec=="A_OPUS"{fvid_media::owned_opus_packet::duration_ns(&packet).unwrap()}
                else {let config=fvid_media::owned_aac::config::AacConfig::parse(&track.codec_private).unwrap();(u64::from(config.frame_samples)*1_000_000_000).div_ceil(u64::from(config.sample_rate))}
            });
            audio.push((p.track,p.pts_ns,duration,p.discard_padding_ns,packet));
        }}
        if c["companions"].as_bool().unwrap(){assert!(!audio.is_empty());}
        fvid_media::transcode_lossless(&source,&output,fvid_media::LosslessTransform{horizontal_flip:true,..Default::default()},&Default::default()).unwrap_or_else(|e|panic!("{}: {e}",c["file"]));
        let mut out=open(&output);
        let video=out.tracks.iter().find(|t|t.kind==1).unwrap().clone();
        if c["rgb"].as_bool().unwrap(){
            assert_eq!(video.colour.matrix,0);
            assert_eq!(video.full_range,Some(c["source"]["full_range"].as_bool().unwrap()));
        }
        let mut decoder=fvid_media::owned_ffv1_decoder::Decoder::from_configuration(video.width as usize,video.height as usize,&video.codec_private,32<<20).unwrap();
        let mut frame=0;let mut actual_audio=Vec::new();
        for i in 0..out.packets.len(){
            let p=out.packets[i].clone();let track=out.tracks.iter().find(|t|t.number==p.track).unwrap().clone();
            let packet=out.read_packet(i).unwrap();
            if track.kind!=1{
                let original=input.tracks.iter().find(|t|t.codec==track.codec).unwrap();
                assert_eq!(track.codec_private,original.codec_private);
                assert_eq!(track.codec_delay_ns,original.codec_delay_ns);
                assert_eq!(track.sample_rate,original.sample_rate);
                assert_eq!(track.channels,original.channels);
                actual_audio.push((original.number,p.pts_ns,p.duration_ns.unwrap_or(track.default_duration_ns),p.discard_padding_ns,packet));continue;
            }
            let decoded=decoder.decode(&packet).unwrap();assert_eq!(decoded.depth,c["depth"].as_u64().unwrap() as u8);
            assert_eq!(p.pts_ns,c["pts_ns"][frame].as_i64().unwrap());
            assert_eq!(p.duration_ns,Some(c["durations_ns"][frame].as_u64().unwrap()));
            let oracle=std::fs::read(root().join(c["source"]["frames"][frame]["display"].as_str().unwrap())).unwrap();
            let w=video.width as usize;let unit=if decoded.depth==8{1}else{2};let mut expected=Vec::new();
            if c["rgb"].as_bool().unwrap(){for row in oracle.chunks_exact(w*3*unit){for pixel in row.chunks_exact(3*unit).rev(){expected.extend_from_slice(pixel)}}}
            else{
                let mono=c["source"]["chroma_format"]==0;let h=video.height as usize;let sub=decoded.frame.subsampling.unwrap();let mut at=0;
                for component in 0..3{if mono&&component>0{for _ in 0..w*h{expected.push(128)}continue;}
                    let [sx,sy]=if component==0{[1,1]}else{sub};let pw=w.div_ceil(sx);let len=pw*h.div_ceil(sy)*unit;
                    for row in oracle[at..at+len].chunks_exact(pw*unit){for sample in row.chunks_exact(unit).rev(){expected.extend_from_slice(sample)}}at+=len;
                }
            }
            assert_eq!(decoded.frame.data,expected,"{} frame {frame}",c["file"]);frame+=1;
        }
        assert_eq!(frame,3);audio.sort();actual_audio.sort();assert_eq!(actual_audio,audio,"audio packet timelines");
        std::fs::remove_file(output).unwrap();
    }
}
