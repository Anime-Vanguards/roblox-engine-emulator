//! Shared temporary directory helper for profile-view integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// A temporary folder of the test's own, removed when dropped
pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new(name: &str) -> Temp {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // (its name always the same length, so a screen that shows a path cuts it at the same place on any machine)
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = ree_profile_view::model::fxhash(format!("{}-{stamp}-{seq}", std::process::id()).as_bytes());
        let dir = std::env::temp_dir().join(format!("ree-profile-view-{name}-{unique:016x}"));
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
