#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(message:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
#[path="batch-159-container-before.rs"] mod avi;
fn main() {
    let root=std::path::Path::new("tests/fixtures/playback-errors/avi-main-header-bounds");
    for codec in ["ima","ms"] {
        for length in [0,16,20] {
            let name=format!("{codec}-short-{length}.avi");let bytes=std::fs::read(root.join(&name)).unwrap();
            let parsed=avi::Avi::parse(&bytes,&Default::default()).unwrap();
            assert_eq!(parsed.streams,2);assert_eq!(parsed.audio()[0].records.len(),3);
            println!("{name}: falsely accepted 2 streams and3 audio records with dwStreams outside the {length}-byte avih");
        }
        for suffix in ["standard","padded"] {
            let name=format!("{codec}-{suffix}.avi");let bytes=std::fs::read(root.join(&name)).unwrap();
            assert_eq!(avi::Avi::parse(&bytes,&Default::default()).unwrap().audio()[0].records.len(),3);
        }
    }
    println!("Six invalid header admissions,four healthy controls. Actual frozen container source;not decoder/worker acceptance.");
}
