//! Shared error contract, owned below both codec and media libraries.
pub use fvid_control::error::{Error, Result};
pub(crate) use fvid_control::error::{invalid, unsupported};
