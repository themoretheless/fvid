use fvid_media::owned_webm::{WebmReader,Limits};
use std::{fs,io::Cursor,path::PathBuf};
fn root()->PathBuf{PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/matroska-text-termination")}
fn check(names:&[&str],name:&str){
 let mut control=WebmReader::open(Cursor::new(fs::read(root().join("control.mkv")).unwrap()),Limits::default()).unwrap();control.scan_all().unwrap();
 for file in names{
  let mut reader=WebmReader::open(Cursor::new(fs::read(root().join(file)).unwrap()),Limits::default()).unwrap();reader.scan_all().unwrap();
  assert_eq!(reader.tracks[0].codec,"V_VP9");assert_eq!(reader.tracks[0].name,name);assert_eq!(reader.packets.len(),3);
  for i in 0..3{assert_eq!(reader.read_packet(i).unwrap(),control.read_packet(i).unwrap());assert_eq!(reader.packets[i].pts_ns,control.packets[i].pts_ns);}
 }
}
#[test]fn nul_terminated_document_type_is_recognized(){check(&["doctype-padding.mkv","doctype-suffix.mkv"],"")}
#[test]fn codec_id_ignores_data_after_its_first_nul(){check(&["codec-suffix.mkv","codec-invalid-suffix.mkv"],"")}
#[test]fn track_name_discards_text_and_invalid_utf8_after_terminator(){check(&["name-suffix.mkv","name-invalid-suffix.mkv"],"Кадр")}
#[test]fn invalid_utf8_before_codec_terminator_is_still_refused(){
 let error=match WebmReader::open(Cursor::new(fs::read(root().join("codec-invalid-prefix.mkv")).unwrap()),Limits::default()){Ok(_)=>panic!("invalid codec prefix accepted"),Err(e)=>e};
 assert_eq!(error.to_string(),"invalid WebM codec ID");
}
#[test]fn lenient_metadata_also_ends_at_its_first_nul(){
 let mut reader=WebmReader::open(Cursor::new(fs::read(root().join("metadata-suffix.mkv")).unwrap()),Limits::default()).unwrap();reader.scan_all().unwrap();
 assert_eq!(reader.writing_app,"fvid");assert_eq!(reader.chapters.len(),1);assert_eq!(reader.chapters[0].title,"Chapter");
}
#[test]fn semantic_termination_keeps_stored_payload_limits_and_prefix_validation(){
 use fvid_media::owned_ebml::Budget;
 for (payload,limit,expected) in [(b"hello\0\xff".as_slice(),7,Some("hello")),(b"\0\xff",2,Some("")),(b"\xff\0",2,None),(b"ok\0padding",3,None)]{
  let mut data=vec![0x85,0x80|payload.len() as u8];data.extend_from_slice(payload);
  let length=data.len() as u64;let mut source=Cursor::new(data);let element=Budget::new(1).next(&mut source,length).unwrap();
  let result=element.read_text(&mut source,limit);
  match expected{Some(expected)=>assert_eq!(result.unwrap(),expected),None=>assert!(result.is_err())}
 }
}
