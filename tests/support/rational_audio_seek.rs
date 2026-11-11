#![allow(dead_code)]
use fvid::audio::{AudioBackend,AudioError,AudioPacket,AudioSpec,AudioStep,AudioStream,DecodedAudio};
use std::{sync::{Arc,Mutex},time::Duration};
#[derive(Clone,Debug)]
pub struct Packet{pub rate:u32,pub channels:u16,pub num:u128,pub den:u128,pub data:Vec<u8>}
fn reduce(mut n:u128,mut d:u128)->(u128,u128){let(original_n,original_d)=(n,d);while d!=0{let r=n%d;n=d;d=r;}(original_n/n,original_d/n)}
impl Packet{
 pub fn from_audio(p:AudioPacket,rate:u32,channels:u16)->Self{assert!(p.timebase_den>0);Self{rate,channels,num:u128::from(p.pts)*u128::from(p.timebase_num),den:u128::from(p.timebase_den),data:p.data}}
 pub fn point(&self,index:usize)->(u128,u128){reduce(self.num*u128::from(self.rate)+index as u128*self.den,self.den*u128::from(self.rate))}
 pub fn frames(&self)->usize{let stride=usize::from(self.channels)*4;assert!(stride>0 && self.data.len().is_multiple_of(stride));self.data.len()/stride}
 pub fn time(&self)->Duration{duration(self.num,self.den)}
}
pub fn duration(n:u128,d:u128)->Duration{Duration::new((n/d)as u64,((n%d)*1_000_000_000/d)as u32)}
pub fn collect(stream:&mut dyn AudioStream)->Vec<Packet>{
 fn emit(stream:&dyn AudioStream,p:AudioPacket,out:&mut Vec<Packet>){out.push(Packet::from_audio(p,stream.sample_rate(),stream.channels()));}
 fn present(stream:&dyn AudioStream,mut f:DecodedAudio,out:&mut Vec<Packet>){
  if let Some(n)=stream.decoded_packet_sample_limit(f.source_pts,f.source_duration).unwrap(){let bytes=n.checked_mul(usize::from(stream.channels())*4).unwrap();assert!(bytes<=f.packet.data.len());f.packet.data.truncate(bytes);}
  if let Some(p)=stream.present_decoded(f.packet,f.source_pts).unwrap(){emit(stream,p,out);}
 }
 let mut decoder=stream.make_decoder().unwrap();let mut out=Vec::new();
 loop{match stream.next_step().unwrap(){
  Some(AudioStep::Encoded(p))=>if let Some(f)=decoder.decode_packet(&p.data,stream.decoder_stamp(&p),p.duration as u64).unwrap(){present(stream,f,&mut out);},
  Some(AudioStep::ReconfigureDecoder)=>{while let Some(f)=decoder.finish_packet().unwrap(){present(stream,f,&mut out);}stream.advance_configuration().unwrap();decoder=stream.make_decoder().unwrap();},
  Some(AudioStep::ResetDecoder)=>stream.reset_decoder(&mut decoder).unwrap(),
  Some(AudioStep::Pcm(p))=>emit(stream,p,&mut out),
  None=>{if !stream.drain_decoded_at_eof(){decoder.reset();break;}if let Some(f)=decoder.finish_packet().unwrap(){present(stream,f,&mut out);continue;}break;}
 }}stream.validate_eof().unwrap();out
}
pub fn suffix(full:&[Packet],target:Duration,end:(u128,u128))->Vec<Packet>{
 let ns=target.as_nanos();if ns*end.1>=end.0*1_000_000_000{return Vec::new();}
 full.iter().filter_map(|p|{
  let delta=(ns*p.den)as i128-(p.num*1_000_000_000)as i128;
  let first=if delta<=0{0}else{((delta as u128*u128::from(p.rate)).div_ceil(1_000_000_000*p.den)).min(p.frames()as u128)as usize};
  (first<p.frames()).then(||{let(num,den)=p.point(first);Packet{rate:p.rate,channels:p.channels,num,den,data:p.data[first*usize::from(p.channels)*4..].to_vec()}})
 }).collect()
}
pub fn anchor(wanted:&[Packet],end:(u128,u128))->Duration{wanted.first().map_or_else(||duration(end.0,end.1),Packet::time)}
pub fn exact(got:&[Packet],wanted:&[Packet],context:&str){
 fn points(packets:&[Packet])->impl Iterator<Item=(u32,u16,u128,u128,&[u8])>{packets.iter().flat_map(|p|p.data.chunks_exact(usize::from(p.channels)*4).enumerate().map(move|(i,b)|{let(n,d)=p.point(i);(p.rate,p.channels,n,d,b)}))}
 let(mut a,mut b)=(points(got),points(wanted));let mut index=0;
 loop{let(x,y)=(a.next(),b.next());if x.is_none()&&y.is_none(){break;}assert_eq!(x,y,"{context}:native sample{index}");index+=1;}
}
#[derive(Default)]pub struct RawTrace{pub packets:Vec<Packet>,pub anchors:Vec<Duration>}
pub struct RawCapture{pub inner:super::transitions::Capture,pub raw:Arc<Mutex<RawTrace>>}
impl AudioBackend for RawCapture{
 fn start(&mut self,s:AudioSpec)->Result<(),AudioError>{self.inner.start(s)}
 fn push(&mut self,p:AudioPacket)->Result<(),AudioError>{let s=self.inner.spec.unwrap();let copy=Packet{rate:s.sample_rate,channels:s.channels,num:u128::from(p.pts)*u128::from(p.timebase_num),den:u128::from(p.timebase_den),data:p.data.clone()};assert!(copy.den>0);self.raw.lock().unwrap().packets.push(copy);self.inner.push(p)}
 fn position(&self)->Duration{self.inner.position()}
 fn flush(&mut self,at:Duration)->Result<(),AudioError>{let mut raw=self.raw.lock().unwrap();raw.packets.clear();raw.anchors.push(at);drop(raw);self.inner.flush(at)}
 fn begin_drain(&mut self)->Result<(),AudioError>{self.inner.begin_drain()}
 fn drain_status(&self)->Result<bool,AudioError>{self.inner.drain_status()}
 fn end_drain(&mut self)->Result<(),AudioError>{self.inner.end_drain()}
 fn pause(&mut self)->Result<(),AudioError>{self.inner.pause()}
 fn resume(&mut self)->Result<(),AudioError>{self.inner.resume()}
 fn stop(&mut self)->Result<(),AudioError>{self.inner.stop()}
 fn reconfigure(&mut self,s:AudioSpec,r:Option<fvid_media::owned_stereo_router::StereoRouter>,at:Duration,p:bool)->Result<(),AudioError>{self.inner.reconfigure(s,r,at,p)}
}
