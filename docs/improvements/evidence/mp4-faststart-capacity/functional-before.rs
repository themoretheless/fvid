extern crate self as fvid;
type Result<T> = std::io::Result<T>;
fn invalid(s:&str)->std::io::Error {std::io::Error::new(std::io::ErrorKind::InvalidData,s)}
mod media_control {
 pub struct CancelFlag;impl CancelFlag {pub fn is_cancelled(&self)->bool{false}}
 #[derive(Clone,Copy,Debug)] pub struct ProgressEvent {pub packets:u64,pub payload_bytes:u64,pub done:bool}
 pub struct ProgressHook;impl ProgressHook {pub fn emit(&self,_:ProgressEvent){}}
}
pub mod container {
 #[path="/private/tmp/fvid-faststart-capacity-audit/original.rs"] pub mod mp4_relocate;
}
#[path="/Users/themoretheless/Documents/Sources/fvid/tests/mp4_relocation_admission.rs"] mod regression;
