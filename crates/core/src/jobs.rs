//! Background jobs and UI messaging.
//!
//! The UI thread owns all sheets; background work (indexing, sorting,
//! aggregation, loading) communicates results back over a channel as `UiMsg`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::sheet::SheetId;
use crate::store::RowId;

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn as_atomic(&self) -> &AtomicBool {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Messages from background jobs to the UI loop.
pub enum UiMsg {
    /// Repaint needed (e.g. more rows indexed).
    Dirty,
    /// A sort finished; swap in the permutation.
    SortDone {
        sheet: SheetId,
        perm: Arc<Vec<RowId>>,
        spec: Vec<(usize, bool)>,
    },
    /// A derived sheet is ready to push.
    SheetReady(Box<crate::sheet::Sheet>),
    /// A background search finished: move the cursor (None = no match).
    SearchDone {
        sheet: SheetId,
        view_idx: Option<u64>,
    },
    Status(Severity, String),
    JobFinished { name: String },
}

/// Handle for spawning cancellable background jobs.
pub struct JobManager {
    tx: Sender<UiMsg>,
    jobs: parking_lot::Mutex<Vec<(String, CancelToken)>>,
}

impl JobManager {
    pub fn new(tx: Sender<UiMsg>) -> Self {
        JobManager {
            tx,
            jobs: parking_lot::Mutex::new(Vec::new()),
        }
    }

    pub fn sender(&self) -> Sender<UiMsg> {
        self.tx.clone()
    }

    /// Spawn a named job on a dedicated thread. The job receives a cancel
    /// token and a message sender.
    pub fn spawn<F>(&self, name: impl Into<String>, f: F) -> CancelToken
    where
        F: FnOnce(CancelToken, Sender<UiMsg>) + Send + 'static,
    {
        let name = name.into();
        let token = CancelToken::new();
        let tok = token.clone();
        let tx = self.tx.clone();
        self.jobs.lock().push((name.clone(), token.clone()));
        std::thread::Builder::new()
            .name(format!("rd-job-{}", name))
            .spawn(move || {
                f(tok, tx.clone());
                let _ = tx.send(UiMsg::JobFinished { name });
            })
            .expect("spawn job thread");
        token
    }

    /// Cancel all outstanding jobs (visidata ^C).
    pub fn cancel_all(&self) {
        for (_, tok) in self.jobs.lock().iter() {
            tok.cancel();
        }
    }

    /// Cancel outstanding jobs with the given name (e.g. supersede a search).
    pub fn cancel_named(&self, name: &str) {
        for (n, tok) in self.jobs.lock().iter() {
            if n == name {
                tok.cancel();
            }
        }
    }

    pub fn reap(&self, finished: &str) {
        self.jobs.lock().retain(|(n, _)| n != finished);
    }

    pub fn n_active(&self) -> usize {
        self.jobs.lock().len()
    }
}
