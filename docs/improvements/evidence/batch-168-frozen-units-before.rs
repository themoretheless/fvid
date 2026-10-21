#![allow(dead_code)]
type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
fn invalid(s:&str)->Box<dyn std::error::Error+Send+Sync>{std::io::Error::new(std::io::ErrorKind::InvalidData,s).into()}
#[path="batch-168-avi-before.rs"]mod avi;
