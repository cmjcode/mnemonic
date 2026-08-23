//! Basic external-change file watcher for the vault (§3.1.1, §6 risk
//! "Sinkronisasi File Eksternal"). Fase 1 scope: detect changes and
//! signal "please rescan" — full conflict-resolution dialog is deferred.
//! Callers: `app.rs` (owns the watcher, polls `try_recv_rescan`).

use std::path::Path;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Debounce window: multiple filesystem events within this window collapse
/// into a single rescan signal, avoiding rescan storms on bulk saves.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Watches a vault root for external `.md` changes and coalesces bursts
/// of events into a single debounced "rescan needed" signal.
pub struct VaultWatcher {
    _inner: RecommendedWatcher, // kept alive so the OS watch stays active
    events: Receiver<()>,
    pending_since: Option<Instant>,
}

impl VaultWatcher {
    pub fn watch(root: &Path) -> Result<VaultWatcher> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                // Ignore send errors: the receiving end may have been
                // dropped if the app is shutting down.
                let _ = tx.send(());
            }
        })
        .context("creating file watcher")?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("watching vault root {}", root.display()))?;

        Ok(VaultWatcher {
            _inner: watcher,
            events: rx,
            pending_since: None,
        })
    }

    /// Call periodically (e.g. once per UI frame). Returns `true` exactly
    /// once per debounced burst of external changes, signaling the caller
    /// should rescan the vault.
    pub fn poll_rescan_needed(&mut self) -> bool {
        loop {
            match self.events.try_recv() {
                Ok(()) => self.pending_since = Some(Instant::now()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if let Some(since) = self.pending_since {
            if since.elapsed() >= DEBOUNCE {
                self.pending_since = None;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_external_file_change_after_debounce() {
        let dir = tempdir().unwrap();
        let mut watcher = VaultWatcher::watch(dir.path()).unwrap();

        std::fs::write(dir.path().join("external.md"), "isi baru").unwrap();

        // Give the OS watcher time to deliver the event, then wait out
        // the debounce window.
        std::thread::sleep(Duration::from_millis(200));
        let _ = watcher.poll_rescan_needed(); // may register pending, not yet due
        std::thread::sleep(DEBOUNCE + Duration::from_millis(100));

        assert!(watcher.poll_rescan_needed());
        // Second immediate poll should be false: burst already consumed.
        assert!(!watcher.poll_rescan_needed());
    }
}
