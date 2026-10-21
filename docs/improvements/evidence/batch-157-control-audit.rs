#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(message:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
mod audio {
 pub struct AudioPacket {pub data:Vec<u8>,pub pts:u64,pub timebase_num:u32,pub timebase_den:u32}
 pub trait AudioDecode:Send {
  fn decode_encoded(&mut self,data:&[u8],pts:u64,duration:u64)->crate::Result<Option<AudioPacket>>;
  fn reset(&mut self);
 }
}
#[path="batch-156-container-candidate.rs"]pub mod xm_model;
mod container {pub use crate::xm_model as xm;}
#[path="batch-152-xm-decoder-before.rs"]mod historical;
#[path="batch-157-decoder-before.rs"]mod before;
#[path="batch-157-decoder-candidate.rs"]mod after;
fn render(bytes:&[u8],rate:u32,channels:u16,profile:u8)->Vec<u8>{
 let module=container::xm::Module::parse(bytes,&Default::default()).unwrap();
 let mut decoder:Box<dyn audio::AudioDecode>=match profile {0=>Box::new(historical::XmDecoder::new(bytes,rate,channels).unwrap()),1=>Box::new(before::XmDecoder::new(bytes,rate,channels).unwrap()),_=>Box::new(after::XmDecoder::new(bytes,rate,channels).unwrap())};
 let mut pcm=Vec::new();
 for index in 0..module.packets(){
  let mut data=module.packet(index);
  if index==module.timeline().len(){for cell in data[container::xm::ROW_HEADER_BYTES..].chunks_exact_mut(container::xm::CELL_BYTES){cell[0]=container::xm::NOTE_KEY_OFF;}}
  let(start,length)=module.packet_time(index);let pts=start/1000;let duration=(start+length)/1000-pts;
  let packet=decoder.decode_encoded(&data,pts,duration).unwrap().unwrap();assert_eq!(packet.timebase_den,rate);pcm.extend(packet.data);
 }
 pcm
}
fn main(){
 let root=std::path::Path::new("tests/fixtures/playback-errors/xm-invalid-defaults");
 let names=["slot-129-note-mute","slot-129-selection-mute","slot-129-note-pan","slot-129-key-off","slot-255-note-mute","slot-255-selection-mute","slot-255-note-pan","slot-255-key-off","populated-note-defaults-control","populated-selection-defaults-control","valid-empty-slot-defaults-control","valid-empty-slot-key-off-control","zero-instrument-control","default-level-invalid-control"];
 let(mut historical_bad,mut current_bad,mut accepted)=(0,0,0);
 for name in names {
  let bytes=std::fs::read(root.join(format!("{name}.xm"))).unwrap();let control=std::fs::read(root.join(format!("{name}-control.xm"))).unwrap();
  for rate in [11025,44100] {for channels in [1,2] {
   let bad=name.starts_with("slot-")&&(channels==2||!name.ends_with("-note-pan"));
   let old_bad=bad&&!name.ends_with("-key-off");
   let historical=render(&bytes,rate,channels,0);let historical_control=render(&control,rate,channels,0);
   assert_eq!(historical!=historical_control,old_bad,"historical {name} rate{rate} channels{channels}");historical_bad+=usize::from(old_bad);
   let before=render(&bytes,rate,channels,1);let before_control=render(&control,rate,channels,1);
   assert_eq!(before!=before_control,bad,"before {name} rate{rate} channels{channels}");current_bad+=usize::from(bad);
   let after=render(&bytes,rate,channels,2);let after_control=render(&control,rate,channels,2);
   assert!(before_control==after_control,"control changed by fix:{name} rate{rate} channels{channels}");
   assert!(after==after_control,"after {name} rate{rate} channels{channels}");accepted+=1;
  }}
  println!("{name}:historical/current classifications verified;four exact candidate pairs and four unchanged current controls");
 }
 assert_eq!((historical_bad,current_bad,accepted),(20,28,56));
 println!("20 historical pre-key-off-fix PCM failures,28 immediate-before failures,56 exact after pairs,56 current control streams unchanged. Six stereo mute/pan faults predate FVID0161;two K00 variants expand coverage and add no extra ID. Standalone adapter evidence;Cargo and actual worker gates remain separate.");
}
