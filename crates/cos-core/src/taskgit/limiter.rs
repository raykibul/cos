//! Caps on how many task workers run at once: per workspace and in total.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Parallel workers in one workspace. Each runs an agent and its tests.
pub const DEFAULT_WORKERS_PER_WORKSPACE: usize = 3;
/// Parallel workers across all workspaces.
pub const DEFAULT_WORKERS_TOTAL: usize = 6;

/// Hands out a [`WorkerPermit`] per running worker. A worker that can't get
/// one waits; dropping the permit frees the slot.
pub struct WorkerLimiter {
    per_workspace: usize,
    total: Arc<Semaphore>,
    workspaces: Mutex<HashMap<i64, Arc<Semaphore>>>,
}

/// Held for as long as a worker runs.
#[derive(Debug)]
pub struct WorkerPermit {
    _workspace: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
}

impl Default for WorkerLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_WORKERS_PER_WORKSPACE, DEFAULT_WORKERS_TOTAL)
    }
}

impl WorkerLimiter {
    /// A cap of 0 is treated as 1, so work always makes progress.
    pub fn new(per_workspace: usize, total: usize) -> Self {
        Self {
            per_workspace: per_workspace.max(1),
            total: Arc::new(Semaphore::new(total.max(1))),
            workspaces: Mutex::new(HashMap::new()),
        }
    }

    /// Waits for a slot in the workspace, then one overall. Taking them in
    /// that order means a worker queued behind its own workspace's cap never
    /// holds an overall slot another workspace could use.
    pub async fn acquire(&self, workspace_id: i64) -> WorkerPermit {
        let workspace = self.workspace(workspace_id);
        let workspace = workspace
            .acquire_owned()
            .await
            .expect("the limiter never closes its semaphores");
        let total = self
            .total
            .clone()
            .acquire_owned()
            .await
            .expect("the limiter never closes its semaphores");
        WorkerPermit {
            _workspace: workspace,
            _total: total,
        }
    }

    /// A permit if one is free right now.
    pub fn try_acquire(&self, workspace_id: i64) -> Option<WorkerPermit> {
        let workspace = self.workspace(workspace_id).try_acquire_owned().ok()?;
        let total = self.total.clone().try_acquire_owned().ok()?;
        Some(WorkerPermit {
            _workspace: workspace,
            _total: total,
        })
    }

    /// How many more workers the workspace could start now.
    pub fn available(&self, workspace_id: i64) -> usize {
        self.workspace(workspace_id)
            .available_permits()
            .min(self.total.available_permits())
    }

    fn workspace(&self, id: i64) -> Arc<Semaphore> {
        self.workspaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_workspace)))
            .clone()
    }
}
