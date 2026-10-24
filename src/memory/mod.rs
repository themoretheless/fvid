//! Memory management optimizations: pooling, arenas, and interning.
//! 
//! This module provides infrastructure for reducing heap allocations
//! and improving cache locality through various memory optimization techniques.
//! 
//! # Components
//! 
//! - **pool**: Generic object pooling with acquire/release lifecycle
//! - **string_interner**: Thread-safe string deduplication via Arc<str>
//! - **arena**: Single large allocation for frame-level temporaries
//! - **motion_pool**: Motion vector pools for HEVC temporal prediction
//! - **cu_state_pool**: CU state descriptor pools for HEVC partition trees

pub mod pool;
mod string_interner;

pub use pool::*;
pub use string_interner::{StringInterner, InternedString};
