//! Shared media cancellation, progress and operation policies, independent of codecs.
#![forbid(unsafe_code)]

#[derive(Clone, Debug)]
pub struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl CancelFlag {
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Default for CancelFlag {
    fn default() -> Self {
        Self::new()
    }
}

/// Cooperative progress sample between packets (and a final `done` event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressEvent {
    pub packets: u64,
    pub payload_bytes: u64,
    pub done: bool,
}

#[derive(Clone)]
pub struct ProgressHook(std::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>);
impl ProgressHook {
    pub fn new<F>(hook: F) -> Self
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        Self(std::sync::Arc::new(hook))
    }
    pub fn emit(&self, event: ProgressEvent) {
        (self.0)(event);
    }
}
impl std::fmt::Debug for ProgressHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressHook(..)")
    }
}

#[derive(Clone, Debug)]
pub struct CopyOptions {
    /// Empty selects every stream; otherwise indices are preserved in this order.
    pub streams: Vec<usize>,
    pub max_packet_bytes: usize,
    /// Optional hard stop after this many operation-specific packets.
    /// A decoder counts selected compressed packets, including preroll; a muxer
    /// counts copied packets. Multi-phase adapters apply the limit to input work.
    pub max_packets: Option<u64>,
    /// Optional admission limit on estimated decoder DPB + Fvid scratch bytes.
    /// Not a promise of peak process RSS.
    pub max_controlled_bytes: Option<usize>,
    /// Optional process RSS limit probed every 256 packets (and at start when set).
    pub max_rss_bytes: Option<u64>,
    /// Cooperative cancellation checked between packets.
    pub cancel: Option<CancelFlag>,
    /// Optional progress hook; sampling cadence is operation-specific.
    /// Successful completion emits `done=true` after output publication when applicable.
    pub progress: Option<ProgressHook>,
    /// Container-level tags to set after copying source metadata (`KEY=VALUE`).
    pub metadata_set: Vec<(String, String)>,
    /// Container-level tag keys to delete after the copy.
    pub metadata_delete: Vec<String>,
    /// Source-stream tags to set: `(stream_index, key, value)`.
    pub stream_metadata_set: Vec<(usize, String, String)>,
    /// Source-stream tags to delete: `(stream_index, key)`.
    pub stream_metadata_delete: Vec<(usize, String)>,
}
impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            streams: vec![],
            max_packet_bytes: 64 * 1024 * 1024,
            max_packets: None,
            max_controlled_bytes: None,
            max_rss_bytes: None,
            cancel: None,
            progress: None,
            metadata_set: vec![],
            metadata_delete: vec![],
            stream_metadata_set: vec![],
            stream_metadata_delete: vec![],
        }
    }
}

pub mod error;
