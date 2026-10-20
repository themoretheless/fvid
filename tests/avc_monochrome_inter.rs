use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn monochrome_inter_reference_prediction() {
 let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avc-monochrome-inter");
 let cases:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
 for case in cases.as_array().unwrap(){
  let name=case["name"].as_str().unwrap();let mut decoder=AvcDecoder::new(&std::fs::read(root.join(format!("{name}.avcc"))).unwrap(),1<<20).unwrap();
  let raw=std::fs::read(root.join(format!("{name}.y16"))).unwrap();let samples:Vec<u16>=raw.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
  for _ in 0..2 {
   for i in 0..2 {
    let picture=decoder.decode_order(&std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap()).unwrap_or_else(|e|panic!("{name} frame{i}: {e}")).unwrap();
    assert_eq!(picture.y,samples[i*512..(i+1)*512],"{name} frame{i}");assert!(picture.cb.is_empty()&&picture.cr.is_empty());
   }
   decoder.reset();
  }
  let mut reader=fvid::playback::video::mp4::Mp4VideoReader::open(std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mp4"))).unwrap()),fvid::container::mp4::Limits::default(),16<<20).unwrap();
  for _ in 0..2 {
   for i in 0..2 {
    let frame=reader.read_frame().unwrap_or_else(|e|panic!("MP4 {name}: {e}")).unwrap();let packed=frame.packed.unwrap();let depth=case["depth"].as_u64().unwrap()as u8;
    assert_eq!(packed.depth,depth);assert_eq!(packed.frame.subsampling,Some([2,2]));let mut expected=Vec::new();
    for v in samples[i*512..(i+1)*512].iter().copied().chain(std::iter::repeat_n(1u16<<(depth-1),256)){if depth==8 {expected.push(v as u8);}else{expected.extend_from_slice(&v.to_le_bytes());}}
    assert_eq!(packed.frame.data,expected,"MP4 {name} frame{i}");
   }
   assert!(reader.read_frame().unwrap().is_none());reader.rewind();
  }
 }
}
