#![allow(dead_code,unused_imports)]
extern crate self as fvid_control;
pub mod error{pub type Error=Box<dyn std::error::Error+Send+Sync>;pub type Result<T>=std::result::Result<T,Error>;pub fn invalid(m:&str)->Error{std::io::Error::new(std::io::ErrorKind::InvalidData,m).into()}}
#[path="batch-163-ogg-candidate.rs"]mod ogg;
fn main(){
 let root=std::path::Path::new("tests/fixtures/playback-errors/ogg-continuation-boundary");
 for(name,parts)in[("one-page-padded",vec!["padded20.packet"]),("split-padded",vec!["padded20.packet"]),("completed-prefix",vec!["padded20.packet"]),("zero-terminator",vec!["exact255.packet"]),("parent-one-page",vec!["parent40.packet","raw20.packet"]),("parent-three-pages",vec!["parent40.packet","raw20.packet"]),("empty-bridge",vec!["padded20.packet"]),("plain-ten",vec!["raw10.packet"]),("joined-tail",vec!["raw20.packet"]),("joined-two-pages",vec!["raw20.packet"]),("joined-zero-terminator",vec!["raw20.packet"]) ]{
  let b=std::fs::read(root.join(format!("{name}.ogg"))).unwrap();let p=ogg::Ogg::parse(&b,&Default::default()).unwrap();let out=&p.chains()[0].packets[2..];assert_eq!(out.len(),parts.len(),"{name}");for(a,e)in out.iter().zip(parts){assert_eq!(a.data,std::fs::read(root.join(e)).unwrap(),"{name}");}
 }
 for name in["missing-final-prefix","missing-middle","missing-zero-terminator-prefix"]{let b=std::fs::read(root.join(format!("{name}.ogg"))).unwrap();let e=ogg::Ogg::parse(&b,&Default::default()).unwrap_err().to_string();assert!(e.contains("continuation"),"{name}:{e}");}
 let counter=root.parent().unwrap().join("ogg-page-sequence");for name in["lost-first-audio","gap","duplicate","backward"]{let b=std::fs::read(counter.join(format!("{name}.ogg"))).unwrap();let e=ogg::Ogg::parse(&b,&Default::default()).unwrap_err().to_string();assert!(e.contains("sequence"));}
 println!("Candidate source type-checks and preserves8 controls;3 joined fragments discard,3 missing flags refuse,and4 counter failures remain refused. Standalone core preflight only;production source not changed and Cargo/runtime gates still required.");
}
