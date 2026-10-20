use std::{fs,io::Cursor,path::{Path,PathBuf}};
fn root()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ffv1-reordered-trim")}
fn frames(path:&Path)->Vec<(i64,Vec<u8>)>{
 let mut reader=fvid_media::owned_webm::WebmReader::open(Cursor::new(fs::read(path).unwrap()),Default::default()).unwrap();reader.scan_all().unwrap();
 let t=reader.tracks[0].clone();let mut d=fvid_media::owned_ffv1_decoder::Decoder::from_configuration(t.width as usize,t.height as usize,&t.codec_private,32<<20).unwrap();
 (0..reader.packets.len()).map(|i|{let pts=reader.packets[i].pts_ns;let frame=d.decode(&reader.read_packet(i).unwrap()).unwrap();(pts,frame.frame.data)}).collect()
}
#[test]
fn fixture_reorders_only_independently_decodable_frames(){
 let control=frames(&root().join("control.mkv"));let reordered=frames(&root().join("reordered.mkv"));
 assert_eq!(control.len(),3);assert_eq!(reordered.iter().map(|f|f.0).collect::<Vec<_>>(),[0,40_000_000,20_000_000]);
 for (i,j) in [(0,0),(1,2),(2,1)]{assert_eq!(control[i],reordered[j]);}
 assert_ne!(control[0].1,control[1].1);
}
fn export(interval:Option<(i64,i64)>){
 let mut outputs=Vec::new();
 for name in ["control","reordered"]{
  let output=std::env::temp_dir().join(format!("fvid-reordered-ffv1-{}-{name}-{}.mkv",std::process::id(),interval.map_or(0, |(from,to)| from * 1_000_000 + to)));
  fvid_media::transcode_lossless(&root().join(format!("{name}.mkv")),&output,fvid_media::LosslessTransform{horizontal_flip:true,interval,..Default::default()},&Default::default()).unwrap();
  outputs.push(frames(&output));fs::remove_file(output).unwrap();
 }
 let expected = [0i64,20_000,40_000].into_iter().filter(|&pts| interval.is_none_or(|(from,to)| pts >= from && pts < to)).count();
 assert_eq!(outputs[0].len(),expected);
 assert_eq!(outputs[1],outputs[0],"cutoff in packet order must not discard a later stored in-range frame");
}
#[test]fn full_export_presents_every_frame_in_timestamp_order(){export(None)}
#[test]fn trim_retains_in_range_frames_stored_after_out_of_range_packets(){export(Some((0,30_000)))}

#[test]fn trim_boundaries_and_nonzero_starts_match_ordered_controls(){
 for interval in [(0,20_000),(20_000,40_000),(10_000,30_000),(30_000,50_000)]{export(Some(interval));}
}
#[test]
fn reading_past_an_out_of_range_packet_still_obeys_the_packet_budget(){
 let output=std::env::temp_dir().join(format!("fvid-reordered-ffv1-budget-{}.mkv",std::process::id()));
 let error=fvid_media::transcode_lossless(&root().join("reordered.mkv"),&output,fvid_media::LosslessTransform{horizontal_flip:true,interval:Some((0,30_000)),..Default::default()},&fvid_control::CopyOptions{max_packets:Some(2),..Default::default()}).unwrap_err();
 assert!(error.to_string().contains("packet count exceeds limit"),"{error}");
 assert!(!output.exists(),"failed export must not publish a partial result");
}
