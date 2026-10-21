#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(message:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
#[path="batch-165-avi-candidate.rs"] mod avi;
fn main(){
 let root=std::path::Path::new("tests/fixtures/playback-errors/avi-riff-boundary");
 for codec in ["ima","ms"]{
  let base=std::fs::read(root.join(format!("{codec}-complete.avi"))).unwrap();let movi=base.windows(4).position(|v|v==b"movi").unwrap()-8;
  for size in [0,2,4,movi-8]{let name=format!("{codec}-short-{size}.avi");let b=std::fs::read(root.join(&name)).unwrap();let e=avi::Avi::parse(&b,&Default::default()).unwrap_err().to_string();assert!(e.contains("RIFF")||e.contains("no header list")||e.contains("no movi list"),"{name}:{e}");println!("{name}: refused at the declared RIFF boundary: {e}");}
  for suffix in ["complete","overdeclared","sentinel"]{let b=std::fs::read(root.join(format!("{codec}-{suffix}.avi"))).unwrap();assert_eq!(avi::Avi::parse(&b,&Default::default()).unwrap().audio()[0].records.len(),3);}
  let name=format!("{codec}-outside-index.avi");let b=std::fs::read(root.join(&name)).unwrap();let p=avi::Avi::parse(&b,&Default::default()).unwrap();assert_eq!(p.audio()[0].records.len(),3);println!("{name}: complete first RIFF ignores its outside index");
 }
 println!("Candidate preflight: eight specific refusals,two outside-index acceptances,six preserved controls. Native gates still required;production source not changed.");
}
