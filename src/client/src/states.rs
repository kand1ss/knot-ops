use crate::handles::*;
use knot_sys::process::PlatformHandle;

#[non_exhaustive]
pub enum DaemonSession {
    Ready(ControlHandle),
    Unsynced(UnsyncedHandle),
}

#[non_exhaustive]
#[derive(Debug)]
pub enum ConnectState {
    Offline(OfflineHandle),
    Stale(StaleHandle),
    Connected(ConnectedHandle),
    Hung(KillHandle<PlatformHandle>),
}
