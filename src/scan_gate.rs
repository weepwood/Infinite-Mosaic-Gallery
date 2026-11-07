//! Coalesces refresh and filesystem notifications while a directory scan runs.
//! The scanner itself remains on the background executor.
#[derive(Default, Debug)]
pub struct ScanGate {
    active: bool,
    dirty: bool,
}

impl ScanGate {
    /// Returns true when the caller should start a new scan immediately.
    /// Opening a different folder supersedes any in-flight scan; a refresh
    /// while active becomes exactly one additional scan.
    pub fn request(&mut self, preserve_view: bool) -> bool {
        if self.active && preserve_view {
            self.dirty = true;
            false
        } else {
            self.active = true;
            self.dirty = false;
            true
        }
    }

    /// Returns true when one queued refresh should now be started.
    pub fn complete(&mut self) -> bool {
        self.active = false;
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_many_events_into_one_followup_scan() {
        let mut gate = ScanGate::default();
        assert!(gate.request(false));
        for _ in 0..1000 {
            assert!(!gate.request(true));
        }
        assert!(gate.complete());
        assert!(gate.request(true));
        assert!(!gate.complete());
    }

    #[test]
    fn switching_folder_discards_pending_refresh_of_old_folder() {
        let mut gate = ScanGate::default();
        assert!(gate.request(false));
        assert!(!gate.request(true));
        assert!(gate.request(false));
        assert!(!gate.complete());
    }

    #[test]
    fn idle_refresh_runs_immediately() {
        let mut gate = ScanGate::default();
        assert!(gate.request(true));
        assert!(!gate.complete());
        assert!(gate.request(true));
    }
}
