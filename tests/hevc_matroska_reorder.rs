use std::{io::Cursor,path::PathBuf};
fn root()->PathBuf{PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")}
fn raw_bytes(raw:fvid::playback_native::RawFrame)->Vec<u8>{match raw{
    fvid::playback_native::RawFrame::Planar(p)=>p.frame.data.clone(),
    fvid::playback_native::RawFrame::Planar8(p)=>[p.y.as_slice(),p.cb.as_slice(),p.cr.as_slice()].concat(),
    fvid::playback_native::RawFrame::Avc{picture,..}=>{let mut b=Vec::new();picture.write_planar(&mut b).unwrap();b},
    _=>panic!("software planar payload"),
}}
#[test]
fn hevc_ipb_export_follows_presentation_order_and_infers_visible_durations(){
 let m:serde_json::Value=serde_json::from_slice(&std::fs::read(root().join("hevc-matroska-reorder.json")).unwrap()).unwrap();
 assert_eq!(m["cases"].as_array().unwrap().len(),16);
 for (index,c) in m["cases"].as_array().unwrap().iter().enumerate(){
    assert!(c["decode_to_display"].as_array().unwrap().windows(2).any(|w|w[0].as_u64()>w[1].as_u64()));
    let path=root().join(c["file"].as_str().unwrap());let data=std::fs::read(&path).unwrap();
    let mut coded=fvid_media::owned_webm::WebmReader::open(Cursor::new(&data),Default::default()).unwrap();coded.scan_all().unwrap();
    let config=coded.tracks.iter().find(|t|t.kind==1).unwrap().codec_private.clone();
    let header_decoder=fvid::codec::hevc_decoder::HevcDecoder::from_configuration(&config,32<<20).unwrap();
    let mut has_b=false;
    for i in 0..coded.packets.len(){if coded.packets[i].track==1{
        has_b |= header_decoder.slice_headers(&coded.read_packet(i).unwrap()).unwrap().iter().any(|h|h.slice_type==fvid::codec::hevc_cabac::SliceType::B);
    }}
    assert!(has_b,"fixture must contain actual B slices");
    let mut player=fvid::playback_native::NativeReader::software(Cursor::new(&data),32<<20).unwrap();
    for frame in c["frames"].as_array().unwrap(){assert_eq!(raw_bytes(player.read_frame_raw().unwrap().unwrap()),std::fs::read(root().join(frame.as_str().unwrap())).unwrap(),"authored wrapper must reproduce the IPB pixels in presentation order");}
    assert!(player.read_frame_raw().unwrap().is_none());
    for trim in [false,true] {
    if trim && c["companions"].as_bool().unwrap(){continue;}
    let selected:Vec<usize>=(0..c["frames"].as_array().unwrap().len()).filter(|&i|!trim||(67_000_000..190_000_000).contains(&c["pts_ns"][i].as_i64().unwrap())).collect();
    let out=std::env::temp_dir().join(format!("fvid-hevc-ipb-{}-{index}-{trim}.mkv",std::process::id()));
    fvid_media::transcode_lossless(&path,&out,fvid_media::LosslessTransform{horizontal_flip:true,interval:trim.then_some((67_000,190_000)),..Default::default()},&Default::default()).unwrap_or_else(|e|panic!("{}: {e}",c["file"]));
    let mut reader=fvid_media::owned_webm::WebmReader::open(Cursor::new(std::fs::read(&out).unwrap()),Default::default()).unwrap();reader.scan_all().unwrap();
    let track=reader.tracks.iter().find(|t|t.kind==1).unwrap().clone();
    let mut decoder=fvid_media::owned_ffv1_decoder::Decoder::from_configuration(track.width as usize,track.height as usize,&track.codec_private,32<<20).unwrap();
    let mut frame=0;
    for i in 0..reader.packets.len(){let p=reader.packets[i].clone();if p.track!=track.number{continue;}
      let source_index=selected[frame];
      assert_eq!(p.pts_ns,c["pts_ns"][source_index].as_i64().unwrap()-if trim{67_000_000}else{0},"{}: presentation clock",c["file"]);
      assert_eq!(p.duration_ns,Some(c["durations_ns"][source_index].as_u64().unwrap()),"{}: visible duration",c["file"]);
      let actual=decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
      let gold=std::fs::read(root().join(c["frames"][source_index].as_str().unwrap())).unwrap();let unit=if actual.depth==8{1}else{2};let mut expected=Vec::new();let mut at=0;
      for (w,h) in [(track.width as usize,track.height as usize),(track.width as usize/2,track.height as usize/2),(track.width as usize/2,track.height as usize/2)]{
       let len=w*h*unit;for row in gold[at..at+len].chunks_exact(w*unit){for s in row.chunks_exact(unit).rev(){expected.extend_from_slice(s)}}at+=len;
      }
      assert_eq!(actual.frame.data,expected,"{}: presentation pixels {frame}",c["file"]);frame+=1;
    }
    assert_eq!(frame,selected.len());std::fs::remove_file(out).unwrap();
    }
 }
}
