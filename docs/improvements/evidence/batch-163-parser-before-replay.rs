#![allow(dead_code,unused_imports)]
extern crate self as fvid_control;
pub mod error {pub type Error=Box<dyn std::error::Error+Send+Sync>;pub type Result<T>=std::result::Result<T,Error>;pub fn invalid(m:&str)->Error{std::io::Error::new(std::io::ErrorKind::InvalidData,m).into()}}
#[path="batch-163-ogg-before.rs"]mod ogg;
fn main(){
    let root=std::path::Path::new("tests/fixtures/playback-errors/ogg-continuation-boundary");
    for (name,parts) in [
        ("one-page-padded",vec!["padded20.packet"]),("split-padded",vec!["padded20.packet"]),("completed-prefix",vec!["padded20.packet"]),("zero-terminator",vec!["exact255.packet"]),
        ("parent-one-page",vec!["parent40.packet","raw20.packet"]),("parent-three-pages",vec!["parent40.packet","raw20.packet"]),("empty-bridge",vec!["padded20.packet"]),("plain-ten",vec!["raw10.packet"]),
        ("missing-final-prefix",vec!["raw10.packet"]),("missing-middle",vec!["parent40.packet","raw20.packet"]),("missing-zero-terminator-prefix",vec!["empty.packet"]),
        ("joined-tail",vec!["raw10.packet","raw20.packet"]),("joined-two-pages",vec!["padded20.packet","raw20.packet"]),("joined-zero-terminator",vec!["empty.packet","raw20.packet"]),
    ]{
        let data=std::fs::read(root.join(format!("{name}.ogg"))).unwrap();let parsed=ogg::Ogg::parse(&data,&Default::default()).unwrap();let packets=&parsed.chains()[0].packets[2..];assert_eq!(packets.len(),parts.len(),"{name}");
        for (packet,part) in packets.iter().zip(&parts){assert_eq!(packet.data,std::fs::read(root.join(part)).unwrap(),"{name}");}
        println!("{name}: published packet sizes {:?}",packets.iter().map(|p|p.data.len()).collect::<Vec<_>>());
    }
    println!("Eight complete controls,three missing-flag false admissions and three joined-fragment publications reproduce with the current sequence-aware frozen parser. Not decoder/worker proof.");
}
