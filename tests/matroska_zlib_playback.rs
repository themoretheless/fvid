use fvid::playback_webm::WebmVideoReader;
use std::{fs,io::Cursor,path::{Path,PathBuf}};
fn root()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-zlib")}
fn manifest()->serde_json::Value{serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap()}
#[test]
fn zlib_video_matches_control_pixels_clocks_seek_rewind_and_eof(){
 for case in manifest()["cases"].as_array().unwrap(){
  let mut control=WebmVideoReader::open_software(Cursor::new(fs::read(root().join(case["control"].as_str().unwrap())).unwrap()),32<<20).unwrap();
  let mut frames=Vec::new();while control.read_frame().unwrap(){frames.push((control.dimensions(),control.rgb().to_vec(),control.frame_interval().unwrap()));}assert_eq!(frames.len(),3);
  let mut encoded=WebmVideoReader::open_software(Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),32<<20).unwrap();
  for _ in 0..2{
   for (size,pixels,interval) in &frames{assert!(encoded.read_frame().unwrap());assert_eq!(&encoded.dimensions(),size);assert_eq!(encoded.rgb(),pixels);assert_eq!(&encoded.frame_interval().unwrap(),interval);}
   assert!(!encoded.read_frame().unwrap());encoded.rewind();
  }
  let last=frames.last().unwrap();encoded.seek_to_frame(last.2.0 as i64).unwrap();assert_eq!(encoded.dimensions(),last.0);assert_eq!(encoded.rgb(),last.1);assert!(!encoded.read_frame().unwrap());
 }
}
#[test]
fn zlib_frames_transcode_to_lossless_ffv1_with_the_same_timing_and_pixels(){
 let output=std::env::temp_dir().join(format!("fvid-zlib-export-{}.mkv",std::process::id()));
 fvid_media::transcode_lossless(&root().join("identical-zlib.mkv"),&output,fvid_media::LosslessTransform{horizontal_flip:true,..Default::default()},&Default::default()).unwrap();
 let mut control=WebmVideoReader::open_software(Cursor::new(fs::read(root().join("identical-control.mkv")).unwrap()),32<<20).unwrap();
 let mut actual=WebmVideoReader::open_software(Cursor::new(fs::read(&output).unwrap()),32<<20).unwrap();
 let mut count=0;while control.read_frame().unwrap(){assert!(actual.read_frame().unwrap());assert_eq!(actual.dimensions(),control.dimensions());assert_eq!(actual.rgb(),control.rgb());assert_eq!(actual.frame_interval(),control.frame_interval());count+=1;}assert_eq!(count,3);assert!(!actual.read_frame().unwrap());fs::remove_file(output).unwrap();
}
#[test]
fn identity_copy_preserves_every_encoded_byte(){
 for case in manifest()["cases"].as_array().unwrap(){
  let input=fs::read(root().join(case["file"].as_str().unwrap())).unwrap();let mut output=Vec::new();let stats=fvid_media::owned_matroska_copy::copy(&mut Cursor::new(&input),&mut output,false,None,None).unwrap();
  assert_eq!(stats.packets,if case["file"]=="opus-zlib.mkv"{6}else{3});assert_eq!(output,input);
 }
}
#[test]
fn configured_packet_limit_applies_before_index_decompression_and_publication(){
 let output=std::env::temp_dir().join(format!("fvid-zlib-export-budget-{}.mkv",std::process::id()));
 let mut reader=fvid_media::owned_webm::WebmReader::open(Cursor::new(fs::read(root().join("identical-zlib.mkv")).unwrap()),Default::default()).unwrap();let length=reader.read_packet(0).unwrap().len();
 let error=fvid_media::transcode_lossless(&root().join("identical-zlib.mkv"),&output,fvid_media::LosslessTransform{horizontal_flip:true,..Default::default()},&fvid_control::CopyOptions{max_packet_bytes:length-1,..Default::default()}).unwrap_err();
 assert!(error.to_string().contains("budget"),"{error}");assert!(!output.exists());
}
