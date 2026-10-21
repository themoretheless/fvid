#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(message:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,message).into()}
#[path="batch-156-container-before.rs"] mod xm;
fn main() {
    let root=std::path::Path::new("tests/fixtures/playback-errors/xm-empty-header");
    let mut refused=0;
    for (name,bad) in [("last-29",true),("first-29",true),("middle-29",true),("padded-64",true),("padded-240",true),("empty-only-29",true),("full-241-control",false),("full-243-control",false)] {
        let bytes=std::fs::read(root.join(format!("{name}.xm"))).unwrap();
        let actual=xm::Module::parse(&bytes,&Default::default());
        assert_eq!(actual.is_err(),bad,"{name}: {actual:?}");
        if let Err(error)=actual {assert!(error.to_string().contains("shorter than the 241"));refused+=1;println!("{name}: specific short-header refusal: {error}");}
        let control=std::fs::read(root.join(format!("{name}-control.xm"))).unwrap();
        xm::Module::parse(&control,&Default::default()).unwrap();
    }
    assert_eq!(refused,6);
    for name in ["too-short-28","nonempty-29","truncated-29","huge-header"] {
        let bytes=std::fs::read(root.join(format!("bad-{name}.xm"))).unwrap();
        assert!(xm::Module::parse(&bytes,&Default::default()).is_err(),"{name}");
    }
    println!("Six intended short-header refusals,two accepted full-header cases,eight valid independent controls and four malformed refusals. Standalone actual-parser audit;not Cargo/media/worker acceptance.");
}
