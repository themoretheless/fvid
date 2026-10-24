//! Exact container-time scheduling independent of a decoder's current rate.
use crate::{Error, Result, invalid};
use crate::container::mp4::Edit;
use std::{cmp::Ordering, time::Duration};
include!("../../../crates/fvid-media/src/owned_mp4_edit_clock_impl.rs");
