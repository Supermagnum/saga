//! Media bridge: RoQ/saga-media when `iroh-transport` is enabled; otherwise a no-op stub.

#[cfg(feature = "iroh-transport")]
#[path = "media_bridge_roq.rs"]
mod imp;

#[cfg(feature = "iroh-transport")]
pub use imp::*;

#[cfg(not(feature = "iroh-transport"))]
mod stub {
    pub fn stop_media(_session_id: &str) {}
    pub fn set_muted(_session_id: &str, _muted: bool) {}
    pub fn set_muted_all(_muted: bool) {}
    pub fn set_held(_session_id: &str, _held: bool) {}
    pub fn push_capture(_session_id: &str, _pcm: &[u8]) {}
    pub fn pull_playback(_session_id: &str, out: &mut [u8]) -> usize {
        out.fill(0);
        out.len()
    }
    pub fn media_running(_session_id: &str) -> bool {
        false
    }
    pub fn set_echo_mode(_enabled: bool) {}
    pub fn set_rx_fault(_drop_pct: u32, _jitter_ms: u32) {}
    pub fn on_local_answered(_session_id: &str) -> bool {
        false
    }
    pub fn is_send_armed(_session_id: &str) -> bool {
        false
    }
}
#[cfg(not(feature = "iroh-transport"))]
pub use stub::*;
