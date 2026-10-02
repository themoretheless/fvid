//! Shared FVid Y4M header and bounded line parsing.
use std::io::BufRead;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
fn io_error(error: std::io::Error) -> String {
    error.to_string()
}
include!("owned_y4m_impl.rs");
