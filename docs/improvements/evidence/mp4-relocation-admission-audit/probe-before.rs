type Result<T> = std::io::Result<T>;
fn invalid(s: &str) -> std::io::Error { std::io::Error::new(std::io::ErrorKind::InvalidData,s) }
mod media_control {
 pub struct CancelFlag; impl CancelFlag { pub fn is_cancelled(&self)->bool {false} }
 #[derive(Clone,Copy,Debug)] pub struct ProgressEvent {pub packets:u64,pub payload_bytes:u64,pub done:bool}
 pub struct ProgressHook; impl ProgressHook {pub fn emit(&self,_:ProgressEvent) {}}
}
#[path="/private/tmp/fvid-relocation-admission-audit/before.rs"] mod relocate;
use std::io::{Read,Seek,SeekFrom,Cursor};
struct Count {inner:Cursor<Vec<u8>>, bytes:usize}
impl Read for Count {fn read(&mut self,b:&mut[u8])->std::io::Result<usize>{let n=self.inner.read(b)?;self.bytes+=n;Ok(n)}}
impl Seek for Count {fn seek(&mut self,p:SeekFrom)->std::io::Result<u64>{self.inner.seek(p)}}
fn main(){
 let data=include_bytes!("/Users/themoretheless/Documents/Sources/fvid/tests/fixtures/playback-errors/mp4-relocation-admission/many-boxes.mp4").to_vec();
 let mut r=Count{inner:Cursor::new(data),bytes:0};let mut out=Vec::new();
 let err=relocate::fast_start_with_memory_limit(&mut r,&mut out,None,None,Some(65536)).unwrap_err();
 println!("error={err}; bytes_read={}; output_bytes={}",r.bytes,out.len());
 assert!(err.to_string().contains("controlled memory budget exceeded"));
 assert!(r.bytes<=16,"metadata scan exceeded admission limit: {} bytes",r.bytes);
}
