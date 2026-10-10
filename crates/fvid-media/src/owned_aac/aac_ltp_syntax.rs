//! Owned ordinary/ER AAC LTP side information. No LD syntax or PCM synthesis.
use super::{Result, aac_synthesis::WindowSequence, bits::BitReader, invalid};

include!("aac_ltp_syntax_impl.rs");
