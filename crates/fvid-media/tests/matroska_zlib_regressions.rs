use fvid_media::owned_webm::{WebmReader,Limits};
use std::{fs,io::Cursor,path::PathBuf};
fn root()->PathBuf{PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/matroska-zlib")}
#[test]
fn zlib_control_decodes_authored_three_frame_video(){
 let mut reader=WebmReader::open(Cursor::new(fs::read(root().join("control.mkv")).unwrap()),Limits::default()).unwrap();reader.scan_all().unwrap();
 assert_eq!(reader.packets.len(),3);
 for i in 0..3{let decoded=fvid_codecs::codec::vp9_decoder::Decoder::new(32<<20).decode(&reader.read_packet(i).unwrap()).unwrap();assert_eq!(decoded.picture.size,[[16,16],[32,16],[16,32]][i]);}
}
#[test]
fn zlib_compressed_video_restores_original_codec_packets(){
 let mut control=WebmReader::open(Cursor::new(fs::read(root().join("control.mkv")).unwrap()),Limits::default()).unwrap();control.scan_all().unwrap();
 let mut encoded=WebmReader::open(Cursor::new(fs::read(root().join("plain.mkv")).unwrap()),Limits::default()).unwrap();encoded.scan_all().unwrap();
 assert_eq!(encoded.packets.len(),control.packets.len());
 for i in 0..control.packets.len(){assert_eq!(encoded.read_packet(i).unwrap(),control.read_packet(i).unwrap());assert_eq!(encoded.packets[i].pts_ns,control.packets[i].pts_ns);}
}
fn manifest()->serde_json::Value{serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap()}
#[test]
fn every_zlib_variant_restores_original_packets_clocks_and_read_sizes(){
 for case in manifest()["cases"].as_array().unwrap(){
  let mut encoded=WebmReader::open(Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),Limits::default()).unwrap();encoded.scan_all().unwrap();
  let mut control=WebmReader::open(Cursor::new(fs::read(root().join(case["control"].as_str().unwrap())).unwrap()),Limits::default()).unwrap();control.scan_all().unwrap();
  assert_eq!(encoded.packets.len(),control.packets.len());
  assert!(encoded.estimated_index_payload_bytes().unwrap()>control.estimated_index_payload_bytes().unwrap());
  for i in 0..control.packets.len(){
   let expected=control.read_packet(i).unwrap();assert_eq!(encoded.read_packet(i).unwrap(),expected);assert_eq!(encoded.packet_payload_size(i).unwrap(),expected.len());
   assert_eq!(encoded.packets[i].pts_ns,control.packets[i].pts_ns);assert_eq!(encoded.packets[i].duration_ns,control.packets[i].duration_ns);
   if case["file"]!="opus-zlib.mkv" || encoded.packets[i].track==2{assert!(encoded.packet_read_overhead(i).unwrap()>encoded.packets[i].size);}
   else{assert_eq!(encoded.packet_read_overhead(i).unwrap(),0);}
  }
 }
}
#[test]
fn zlib_opus_lacing_keeps_all_three_tocs_and_exact_silence_pcm(){
 let decode=|name:&str|{let mut output=Vec::new();fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(Cursor::new(fs::read(root().join(name)).unwrap()),&mut output,None,&Default::default()).unwrap();output};
 let expected=decode("opus-control.mkv");assert_eq!(expected.len(),3*960*4);assert_eq!(decode("opus-zlib.mkv"),expected);
}
#[test]
fn corrupted_empty_and_over_budget_streams_are_refused_before_codec_decode(){
 for (file,case) in manifest()["invalid"].as_object().unwrap(){
  let error=match WebmReader::open(Cursor::new(fs::read(root().join(file)).unwrap()),Limits{packet_bytes:case["packet_limit"].as_u64().unwrap() as usize,..Limits::default()}){Ok(_)=>panic!("invalid stream accepted: {file}"),Err(error)=>error};
  assert!(!error.is_unsupported(),"{file}: {error}");assert_eq!(error.to_string(),case["error"].as_str().unwrap(),"{file}");
 }
 let mut control=WebmReader::open(Cursor::new(fs::read(root().join("control.mkv")).unwrap()),Limits{packet_bytes:16_384,..Limits::default()}).unwrap();control.scan_all().unwrap();assert_eq!(control.packets.len(),3);
}
#[test]
fn bounded_inflater_accepts_exact_limit_and_verifies_the_complete_checksum(){
 let original=fs::read(root().join("frame.packet")).unwrap();let stream=fs::read(root().join("frame.zlib")).unwrap();
 for maximum in [original.len(),original.len()+1,usize::MAX]{assert_eq!(fvid_media::owned_matroska_zlib::decode(&stream,maximum).unwrap(),original);}
 assert_eq!(fvid_media::owned_matroska_zlib::decode(&stream,original.len()-1).unwrap_err(),"WebM packet exceeds budget");
 for size in 0..stream.len(){assert!(fvid_media::owned_matroska_zlib::decode(&stream[..size],original.len()).is_err(),"truncation {size}");}
}
#[test]
fn restored_packet_budget_is_checked_before_reading_or_changing_output(){
 use std::{cell::Cell,rc::Rc,io::{Read,Seek,SeekFrom}};
 struct Counter{data:Cursor<Vec<u8>>,reads:Rc<Cell<usize>>}
 impl Read for Counter{fn read(&mut self,out:&mut[u8])->std::io::Result<usize>{let n=self.data.read(out)?;self.reads.set(self.reads.get()+n);Ok(n)}}
 impl Seek for Counter{fn seek(&mut self,from:SeekFrom)->std::io::Result<u64>{self.data.seek(from)}}
 let reads=Rc::new(Cell::new(0));let mut reader=WebmReader::open(Counter{data:Cursor::new(fs::read(root().join("plain.mkv")).unwrap()),reads:reads.clone()},Limits::default()).unwrap();
 let size=reader.packet_payload_size(0).unwrap();assert!(size>reader.packets[0].size);
 reader.restrict_packet_bytes(size);assert_eq!(reader.read_packet(0).unwrap().len(),size);
 reader.restrict_packet_bytes(size-1);let before=reads.get();let mut out=vec![0xa5,0x5a];
 assert_eq!(reader.read_packet_into(0,&mut out).unwrap_err().to_string(),"WebM packet exceeds budget");assert_eq!(out,[0xa5,0x5a]);assert_eq!(reads.get(),before);
}
#[test]
fn audio_controlled_memory_admits_the_additional_inflater_workspace(){
 let minimum=|name:&str|{
  let input=fs::read(root().join(name)).unwrap();
  let fits=|limit|{let mut out=Vec::new();let result=fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(Cursor::new(&input),&mut out,None,&fvid_control::CopyOptions{max_controlled_bytes:Some(limit),..Default::default()});
   match result{Ok(_)=>{assert_eq!(out.len(),3*960*4);true},Err(error)=>{assert!(error.to_string().contains("memory")||error.to_string().contains("budget"),"{error}");false}}
  };
  let (mut low,mut high)=(0,2<<20);assert!(fits(high));while high-low>1{let middle=low+(high-low)/2;if fits(middle){high=middle}else{low=middle}}high
 };
 let control=minimum("opus-control.mkv");let encoded=minimum("opus-zlib.mkv");eprintln!("estimated controlled-memory admission: control={control}, zlib={encoded}, inflater_state={}",fvid_media::owned_matroska_zlib::state_bytes());assert!(encoded>=control+fvid_media::owned_matroska_zlib::state_bytes(),"control {control}, encoded {encoded}");
}
#[test]
fn cancelled_zlib_audio_does_not_emit_pcm(){
 let cancel=fvid_control::CancelFlag::new();cancel.cancel();let mut output=Vec::new();
 let error=fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(Cursor::new(fs::read(root().join("opus-zlib.mkv")).unwrap()),&mut output,None,&fvid_control::CopyOptions{cancel:Some(cancel),..Default::default()}).unwrap_err();
 assert_eq!(error.to_string(),"media operation cancelled");assert!(output.is_empty());
}
