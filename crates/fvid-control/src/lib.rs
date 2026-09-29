//! Shared media cancellation and progress, independent of any codec/backend.
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
