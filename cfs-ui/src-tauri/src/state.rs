use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use cfs_io::volume::CFSVolume;
use cfs_io::fuse::MountHandle;

pub struct OpenVolume {
    pub vol: Arc<CFSVolume>,
    pub path: String,
    pub is_encrypted: bool,
    pub mount_handle: Option<MountHandle>,
    pub drive_letter: Option<String>,
}

pub struct AppState {
    pub volume: Mutex<Option<OpenVolume>>,
    /// Cancellation flag for long-running benchmarks.
    pub bench_cancel: Arc<AtomicBool>,
}
