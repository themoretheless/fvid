//! Keep user-requested playback active when its macOS window loses focus.
//! This does not prevent display or system sleep, and calls no media decoder.
#[cfg(target_os = "macos")]
use objc2::{rc::Retained, runtime::ProtocolObject};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

pub struct PlaybackActivity {
    #[cfg(target_os = "macos")]
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}
impl PlaybackActivity {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            token: NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
                    | NSActivityOptions::LatencyCritical,
                &NSString::from_str("Playing video"),
            ),
        }
    }
}
impl Default for PlaybackActivity {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for PlaybackActivity {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        // SAFETY: this is the unchanged token returned by beginActivity, retained
        // for this guard's lifetime. The non-Clone guard ends it exactly once.
        unsafe {
            NSProcessInfo::processInfo().endActivity(&self.token);
        }
    }
}

/// Mark a playback worker as user-initiated work instead of leaving its
/// scheduling class implicit. Other platforms retain their normal scheduler.
pub fn prioritize_playback_thread() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: the function affects only the calling thread and takes no
        // pointers. 0x19 is QOS_CLASS_USER_INITIATED; zero is a valid relative priority.
        let status = unsafe { pthread_set_qos_class_self_np(0x19, 0) };
        if status != 0 {
            eprintln!("FVid: playback thread QoS request failed ({status})");
        }
    }
}
