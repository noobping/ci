use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;

use crate::error::{CiError, Result};

pub(crate) fn new_run_id() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{stamp}-{}", std::process::id())
}

pub(crate) struct RunLock {
    file: fs::File,
}

impl RunLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().map_err(|_| {
            CiError::Message(format!("another `ci` run is active ({})", path.display()))
        })?;
        Ok(Self { file })
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
