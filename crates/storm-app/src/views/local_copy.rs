//! Bookkeeping for "Download to this PC": copies of torrent files from a remote
//! backend to this machine. Pure state; the details pane drives the transfers.

use std::path::PathBuf;

use storm_proto::TorrentId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyState {
    Queued,
    /// `bytes` comes from watching the partial file grow.
    Running {
        bytes: u64,
    },
    Done,
    Failed(String),
}

impl CopyState {
    pub fn is_active(&self) -> bool {
        matches!(self, CopyState::Queued | CopyState::Running { .. })
    }
}

#[derive(Debug, Clone)]
pub struct CopyJob {
    pub id: u64,
    pub torrent: TorrentId,
    /// Position in `TorrentDetails::files` (for showing state on the row).
    pub pos: usize,
    /// Backend file index (for the download call).
    pub index: usize,
    pub len: u64,
    pub dest: PathBuf,
    pub state: CopyState,
}

impl CopyJob {
    pub fn bytes(&self) -> u64 {
        match self.state {
            CopyState::Done => self.len,
            CopyState::Running { bytes } => bytes.min(self.len),
            _ => 0,
        }
    }
}

/// A file to copy: (position, backend index, length, destination).
pub type CopyRequest = (usize, usize, u64, PathBuf);

#[derive(Default)]
pub struct Copies {
    pub jobs: Vec<CopyJob>,
    next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopySummary {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub active: usize,
    pub bytes: u64,
    pub total_bytes: u64,
}

impl CopySummary {
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            if self.total == 0 { 0.0 } else { (self.done + self.failed) as f64 / self.total as f64 }
        } else {
            self.bytes as f64 / self.total_bytes as f64
        }
    }
}

impl Copies {
    /// Queues copies and returns the new job ids. Finished jobs from a previous
    /// batch are dropped first when nothing is running, so the overall progress
    /// describes the current batch. Files already being copied are not queued twice.
    pub fn enqueue(&mut self, torrent: TorrentId, reqs: Vec<CopyRequest>) -> Vec<u64> {
        if self.summary().active == 0 {
            self.jobs.clear();
        }
        let mut ids = Vec::new();
        for (pos, index, len, dest) in reqs {
            if self.jobs.iter().any(|j| j.torrent == torrent && j.index == index && j.state.is_active()) {
                continue;
            }
            self.jobs.retain(|j| !(j.torrent == torrent && j.index == index));
            self.next_id += 1;
            ids.push(self.next_id);
            self.jobs.push(CopyJob { id: self.next_id, torrent, pos, index, len, dest, state: CopyState::Queued });
        }
        ids
    }

    pub fn get(&self, id: u64) -> Option<&CopyJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn set(&mut self, id: u64, state: CopyState) {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.state = state;
        }
    }

    pub fn for_file(&self, torrent: TorrentId, pos: usize) -> Option<&CopyJob> {
        self.jobs.iter().rev().find(|j| j.torrent == torrent && j.pos == pos)
    }

    /// Marks unfinished jobs as cancelled and returns them (to clean up partial files).
    pub fn cancel(&mut self) -> Vec<CopyJob> {
        let mut out = Vec::new();
        for j in &mut self.jobs {
            if j.state.is_active() {
                out.push(j.clone());
                j.state = CopyState::Failed("Cancelled".into());
            }
        }
        out
    }

    pub fn clear_finished(&mut self) {
        self.jobs.retain(|j| j.state.is_active());
    }

    pub fn summary(&self) -> CopySummary {
        let mut s = CopySummary { total: self.jobs.len(), ..Default::default() };
        for j in &self.jobs {
            match j.state {
                CopyState::Done => s.done += 1,
                CopyState::Failed(_) => s.failed += 1,
                _ => s.active += 1,
            }
            s.bytes += j.bytes();
            s.total_bytes += j.len;
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(i: usize) -> CopyRequest {
        (i, i, 100, PathBuf::from(format!("f{i}")))
    }

    #[test]
    fn summary_tracks_progress() {
        let mut c = Copies::default();
        let ids = c.enqueue(1, vec![req(0), req(1)]);
        c.set(ids[0], CopyState::Done);
        c.set(ids[1], CopyState::Running { bytes: 50 });
        let s = c.summary();
        assert_eq!((s.total, s.done, s.active, s.bytes, s.total_bytes), (2, 1, 1, 150, 200));
        assert!((s.fraction() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn active_files_are_not_queued_twice_and_old_batches_reset() {
        let mut c = Copies::default();
        let a = c.enqueue(1, vec![req(0)]);
        assert!(c.enqueue(1, vec![req(0)]).is_empty());
        c.set(a[0], CopyState::Failed("x".into()));
        let b = c.enqueue(1, vec![req(1)]);
        assert_eq!(b.len(), 1);
        assert_eq!(c.jobs.len(), 1, "finished batch is cleared when idle");
    }

    #[test]
    fn cancel_marks_unfinished() {
        let mut c = Copies::default();
        let ids = c.enqueue(1, vec![req(0), req(1)]);
        c.set(ids[0], CopyState::Done);
        assert_eq!(c.cancel().len(), 1);
        assert_eq!(c.summary().failed, 1);
    }
}
