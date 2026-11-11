//! Shared block/packet-work reporting and control hooks for audio timeline decoding.
use fvid_control::error::{Result, invalid};
use fvid_control::{CopyOptions, ProgressEvent};

pub trait AudioDecodeControl {
    fn check(&self) -> Result<()>;
    fn check_admission(&self, asc: &[u8]) -> Result<()> {
        let _ = asc;
        Ok(())
    }
    fn packet_limit(&self) -> Option<u64>;
    fn packet_limit_reached(&self) -> bool;
    fn packet(&mut self, bytes: usize) -> Result<()>;
}

pub struct CopyProgress<'a> {
    pub options: &'a CopyOptions,
    pub event: ProgressEvent,
    pub check_adts_admission: bool,
}

impl<'a> CopyProgress<'a> {
    pub fn new(options: &'a CopyOptions) -> Self {
        Self {
            options,
            event: ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
            check_adts_admission: false,
        }
    }

    pub fn with_adts_admission(options: &'a CopyOptions) -> Self {
        Self {
            check_adts_admission: true,
            ..Self::new(options)
        }
    }
}

impl AudioDecodeControl for CopyProgress<'_> {
    fn check_admission(&self, asc: &[u8]) -> Result<()> {
        if self.check_adts_admission {
            crate::owned_aac::stream::check_adts_decode_admission(asc, self.options)
        } else {
            Ok(())
        }
    }

    fn check(&self) -> Result<()> {
        if self
            .options
            .cancel
            .as_ref()
            .is_some_and(|c| c.is_cancelled())
        {
            return Err(invalid("media operation cancelled"));
        }
        if self.options.max_packet_bytes == 0 {
            return Err(invalid("packet byte limit must be positive"));
        }
        if self.options.max_packets == Some(0) {
            return Err(invalid("packet work limit requires at least one packet"));
        }
        if self.options.max_controlled_bytes == Some(0) {
            return Err(invalid("controlled memory limit must be positive"));
        }
        crate::owned_budget::check_rss_budget(self.options).map_err(|e| invalid(&e))
    }

    fn packet_limit(&self) -> Option<u64> {
        self.options.max_packets
    }

    fn packet_limit_reached(&self) -> bool {
        self.options
            .max_packets
            .is_some_and(|max| self.event.packets >= max)
    }

    fn packet(&mut self, bytes: usize) -> Result<()> {
        self.event.packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("audio packet count overflow"))?;
        self.event.payload_bytes = self
            .event
            .payload_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| invalid("audio byte count overflow"))?;
        if let Some(hook) = &self.options.progress {
            hook.emit(self.event);
        }
        self.check()
    }
}
