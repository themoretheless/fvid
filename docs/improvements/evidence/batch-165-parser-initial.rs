#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(message:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
#[path="batch-165-avi-before.rs"] mod avi;
fn main(){
 let root=std::path::Path::new("tests/fixtures/playback-errors/avi-riff-boundary");
 for codec in ["ima","ms"]{
  let base=std::fs::read(root.join(format!("{codec}-complete.avi"))).unwrap();let movi=base.windows(4).position(|v|v==b"movi").unwrap()-8;
  for size in [0,2,4,movi-8]{let name=format!("{codec}-short-{size}.avi");let b=std::fs::read(root.join(&name)).unwrap();let p=avi::Avi::parse(&b,&Default::default()).unwrap();assert_eq!(p.streams,2);assert_eq!(p.audio()[0].records.len(),3);println!("{name}: falsely accepted mandatory AVI lists outside declared RIFF extent");}
  for suffix in ["complete","overdeclared","sentinel"]{let b=std::fs::read(root.join(format!("{codec}-{suffix}.avi"))).unwrap();assert_eq!(avi::Avi::parse(&b,&Default::default()).unwrap().audio()[0].records.len(),3);}
  let name=format!("{codec}-outside-index.avi");let b=std::fs::read(root.join(&name)).unwrap();let e=avi::Avi::parse(&b,&Default::default()).unwrap_err().to_string();assert!(e.contains("index says 999"),"{e}");println!("{name}: outside index incorrectly participates in first RIFF validation: {e}");
 }
 println!("Eight false admissions,two outside-index refusals,six existing controls. Parser-only proof from frozen actual source;native PCM/worker gates still required.");
}
