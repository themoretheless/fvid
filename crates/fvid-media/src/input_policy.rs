//! Narrow opt-in demuxer policy for externally callable local-file tools.
use std::cell::Cell;
thread_local! { static STANDALONE: Cell<bool> = const { Cell::new(false) }; }
struct Restore(bool);
impl Drop for Restore {
    fn drop(&mut self) {
        STANDALONE.set(self.0);
    }
}
/// Restrict opens in this thread to standalone media formats. Playlist/manifest
/// demuxers are excluded before stream probing so they cannot follow other files.
/// This does not replace an OS sandbox or defend against concurrent filesystem edits.
pub fn with_standalone_inputs<T>(operation: impl FnOnce() -> T) -> T {
    let _restore = Restore(STANDALONE.replace(true));
    operation()
}
#[cfg_attr(not(feature = "legacy-ffmpeg"), allow(dead_code))]
pub(super) fn active() -> bool {
    STANDALONE.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_scope_restores_nested_state_and_unwind() {
        assert!(!active());
        with_standalone_inputs(|| {
            assert!(active());
            with_standalone_inputs(|| assert!(active()));
            assert!(active());
            let result = std::panic::catch_unwind(|| with_standalone_inputs(|| panic!("fixture")));
            assert!(result.is_err());
            assert!(active());
        });
        assert!(!active());
    }
}
