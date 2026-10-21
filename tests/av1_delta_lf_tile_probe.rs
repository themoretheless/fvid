#[test]
fn inspect() {
 let root=std::path::Path::new("/tmp/fvid-delta-lf-tile-sweep");
 let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("av1-delta-lf-tiles-generated.json")).unwrap()).unwrap();
 for record in manifest["fixtures"].as_array().unwrap(){
 let name=record["file"].as_str().unwrap();let data=std::fs::read(root.join(name)).unwrap();
 let obus:Vec<_>=fvid::codec::av1::Obus::new(&data).map(Result::unwrap).collect();
 let seq=fvid::codec::av1_sequence::Sequence::parse(obus.iter().find(|o|o.kind==1).unwrap().payload).unwrap();
 let frame=obus.iter().find(|o|o.kind==6).unwrap();let h=fvid::codec::av1_frame::Header::parse_intra(&seq,frame.payload,0,0).unwrap();
 println!("{name} levels={:?} delta={:?}",h.filter.levels,h.filter.delta_resolution);
 }
}
