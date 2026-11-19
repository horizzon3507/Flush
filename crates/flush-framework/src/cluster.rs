//! Runs several bot runtimes (one per token) in one process: add, remove,
//! restart, inspect and shut down, with lifecycle events. Mirrors Yuzuha's
//! `YuzuhaCluster`.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use thiserror::Error;

use crate::command::CommandError;
use crate::router::Bot;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClusterError {
    #[error("runtime id must be a non-empty string")]
    EmptyId,
    #[error("runtime {0} already exists")]
    Duplicate(String),
    #[error("runtime {0} does not exist")]
    Unknown(String),
    #[error("runtime {id} failed to start: {message}")]
    Start { id: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    Running,
    /// Its run loop returned without an error.
    Stopped,
    /// Its run loop returned an error (see `last_error`).
    Failed,
}

impl RuntimeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterEvent {
    RuntimeAdded { id: String },
    RuntimeStopped { id: String },
    RuntimeError { id: String, message: String },
    RuntimeRemoved { id: String },
}

/// Snapshot of one runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEntry<M> {
    pub id: String,
    pub status: RuntimeStatus,
    pub last_error: Option<String>,
    pub meta: M,
}

/// Size and per-runtime status, as in Yuzuha's `ClusterRuntimeInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterRuntimeInfo {
    pub size: usize,
    pub runtimes: Vec<(String, RuntimeStatus)>,
}

pub type RuntimeFuture = Pin<Box<dyn Future<Output = Result<(), CommandError>> + Send>>;
type Runner = Arc<dyn Fn() -> Result<RuntimeFuture, CommandError> + Send + Sync>;

struct State {
    status: RuntimeStatus,
    last_error: Option<String>,
}

struct Slot<M> {
    meta: M,
    runner: Runner,
    state: Arc<Mutex<State>>,
    task: Option<async_std::task::JoinHandle<()>>,
}

/// Supervisor for independent bot runtimes keyed by id.
pub struct Cluster<M = ()> {
    slots: BTreeMap<String, Slot<M>>,
    events_tx: UnboundedSender<ClusterEvent>,
    events_rx: UnboundedReceiver<ClusterEvent>,
}

impl<M: Clone> Default for Cluster<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Clone> Cluster<M> {
    pub fn new() -> Self {
        let (events_tx, events_rx) = unbounded();
        Self {
            slots: BTreeMap::new(),
            events_tx,
            events_rx,
        }
    }

    /// Adds a bot runtime. `build` creates the bot now and again on every
    /// [`Cluster::restart`]; keep the token inside it.
    pub fn add<F>(&mut self, id: impl Into<String>, meta: M, build: F) -> Result<(), ClusterError>
    where
        F: Fn() -> Result<Bot, CommandError> + Send + Sync + 'static,
    {
        self.add_runner(id, meta, move || {
            let mut bot = build()?;
            Ok(Box::pin(async move { bot.run().await }) as RuntimeFuture)
        })
    }

    /// Adds a runtime from any run loop. `start` is called now and on every
    /// restart; the future it returns is the runtime.
    pub fn add_runner<F>(
        &mut self,
        id: impl Into<String>,
        meta: M,
        start: F,
    ) -> Result<(), ClusterError>
    where
        F: Fn() -> Result<RuntimeFuture, CommandError> + Send + Sync + 'static,
    {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ClusterError::EmptyId);
        }
        if self.slots.contains_key(&id) {
            return Err(ClusterError::Duplicate(id));
        }
        self.start(id, meta, Arc::new(start))
    }

    fn start(&mut self, id: String, meta: M, runner: Runner) -> Result<(), ClusterError> {
        let future = runner().map_err(|e| ClusterError::Start {
            id: id.clone(),
            message: e.to_string(),
        })?;
        let state = Arc::new(Mutex::new(State {
            status: RuntimeStatus::Running,
            last_error: None,
        }));
        let (task_state, task_events, task_id) =
            (state.clone(), self.events_tx.clone(), id.clone());
        let task = async_std::task::spawn(async move {
            let result = future.await;
            let mut state = task_state.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(()) => {
                    state.status = RuntimeStatus::Stopped;
                    let _ =
                        task_events.unbounded_send(ClusterEvent::RuntimeStopped { id: task_id });
                }
                Err(error) => {
                    let message = error.to_string();
                    state.status = RuntimeStatus::Failed;
                    state.last_error = Some(message.clone());
                    let _ = task_events.unbounded_send(ClusterEvent::RuntimeError {
                        id: task_id,
                        message,
                    });
                }
            }
        });
        self.slots.insert(
            id.clone(),
            Slot {
                meta,
                runner,
                state,
                task: Some(task),
            },
        );
        let _ = self
            .events_tx
            .unbounded_send(ClusterEvent::RuntimeAdded { id });
        Ok(())
    }

    /// Stops and forgets a runtime. `false` when the id is unknown.
    pub async fn remove(&mut self, id: &str) -> bool {
        let Some(mut slot) = self.slots.remove(id) else {
            return false;
        };
        if let Some(task) = slot.task.take() {
            task.cancel().await;
        }
        let _ = self
            .events_tx
            .unbounded_send(ClusterEvent::RuntimeRemoved { id: id.to_owned() });
        true
    }

    /// Stops a runtime and starts it again from its stored builder.
    pub async fn restart(&mut self, id: &str) -> Result<(), ClusterError> {
        let Some(mut slot) = self.slots.remove(id) else {
            return Err(ClusterError::Unknown(id.to_owned()));
        };
        if let Some(task) = slot.task.take() {
            task.cancel().await;
        }
        let _ = self
            .events_tx
            .unbounded_send(ClusterEvent::RuntimeRemoved { id: id.to_owned() });
        self.start(id.to_owned(), slot.meta, slot.runner)
    }

    /// Stops every runtime.
    pub async fn destroy(&mut self) {
        let ids: Vec<String> = self.slots.keys().cloned().collect();
        for id in ids {
            self.remove(&id).await;
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn has(&self, id: &str) -> bool {
        self.slots.contains_key(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.slots.keys().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<ClusterEntry<M>> {
        let slot = self.slots.get(id)?;
        let state = slot.state.lock().unwrap_or_else(|e| e.into_inner());
        Some(ClusterEntry {
            id: id.to_owned(),
            status: state.status,
            last_error: state.last_error.clone(),
            meta: slot.meta.clone(),
        })
    }

    pub fn entries(&self) -> Vec<ClusterEntry<M>> {
        self.slots.keys().filter_map(|id| self.get(id)).collect()
    }

    pub fn info(&self) -> ClusterRuntimeInfo {
        let runtimes: Vec<(String, RuntimeStatus)> = self
            .entries()
            .into_iter()
            .map(|e| (e.id, e.status))
            .collect();
        ClusterRuntimeInfo {
            size: runtimes.len(),
            runtimes,
        }
    }

    /// Next lifecycle event. Pending until one happens.
    pub async fn next_event(&mut self) -> Option<ClusterEvent> {
        self.events_rx.next().await
    }

    /// Lifecycle event already queued, without waiting.
    pub fn try_next_event(&mut self) -> Option<ClusterEvent> {
        self.events_rx.try_recv().ok()
    }
}

impl<M> Drop for Cluster<M> {
    fn drop(&mut self) {
        for slot in self.slots.values_mut() {
            if let Some(task) = slot.task.take() {
                async_std::task::spawn(task.cancel());
            }
        }
    }
}
