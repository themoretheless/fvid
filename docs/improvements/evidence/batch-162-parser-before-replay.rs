#![allow(dead_code,unused_imports)]
extern crate self as fvid_control;
pub mod error {
    pub type Error = Box<dyn std::error::Error+Send+Sync>;
    pub type Result<T> = std::result::Result<T,Error>;
    pub fn invalid(message:&str)->Error{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
}
#[path="batch-162-ogg-before.rs"] mod ogg;
fn main(){
    let root=std::path::Path::new("tests/fixtures/playback-errors/ogg-page-sequence");
    for (name,count) in [("lost-first-audio",4),("gap",5),("duplicate",5),("backward",5)]{
        let data=std::fs::read(root.join(format!("{name}.ogg"))).unwrap();
        let parsed=ogg::Ogg::parse(&data,&Default::default()).unwrap();
        assert_eq!(parsed.pages(),count);assert!(parsed.chains()[0].ended);
        println!("{name}: CRC-valid contradictory page sequence falsely accepted,{} packets",parsed.chains()[0].packets.len());
    }
    for (name,chains,packets) in [("complete-control",1,5),("positive-origin-control",1,4),("legacy-counter-control",1,5),("wrapped-counter-control",1,5),("chained-control",2,10),("interleaved-control",2,10)]{
        let data=std::fs::read(root.join(format!("{name}.ogg"))).unwrap();let parsed=ogg::Ogg::parse(&data,&Default::default()).unwrap();
        assert_eq!(parsed.chains().len(),chains);assert_eq!(parsed.chains().iter().map(|c|c.packets.len()).sum::<usize>(),packets);
    }
    println!("Four false admissions,six preserved controls using frozen actual Ogg parser. Error type adapter only;no decoder/worker acceptance claim.");
}
