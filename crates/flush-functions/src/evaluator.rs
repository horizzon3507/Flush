//! `Evaluator`: walks a parsed `Program` and invokes builtins under the
//! configured limits (mirrors yuzuha's `Evaluator`).

use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ast::{CallNode, Node, Program};
use crate::call::{CallResult, ScriptCall};
use crate::context::ScriptContext;
use crate::errors::{codes, FnError, ScriptError, ScriptFailure};
use crate::limits::Limits;
use crate::registry::FunctionRegistry;
use crate::values;

pub struct Evaluator {
    program: Arc<Program>,
    limits: Limits,
    registry: Arc<RwLock<FunctionRegistry>>,
    calls: AtomicUsize,
    output: AtomicUsize,
    started: Instant,
    /// Extra milliseconds won by `wait` pauses.
    pause: AtomicU64,
    /// Output of the last top-level sequence evaluation (`checkOutput`).
    last_output: Mutex<String>,
}

impl Evaluator {
    pub fn new(
        program: Arc<Program>,
        limits: Limits,
        registry: Arc<RwLock<FunctionRegistry>>,
    ) -> Self {
        Self {
            program,
            limits,
            registry,
            calls: AtomicUsize::new(0),
            output: AtomicUsize::new(0),
            started: Instant::now(),
            pause: AtomicU64::new(0),
            last_output: Mutex::new(String::new()),
        }
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn remaining_ms(&self) -> u64 {
        let elapsed = self.started.elapsed().as_millis() as u64;
        self.limits
            .timeout_ms
            .saturating_add(self.pause.load(Ordering::Relaxed))
            .saturating_sub(elapsed)
    }

    /// `wait` adds back the time spent awaiting external work.
    pub fn wait_started(&self, elapsed: Duration) {
        self.pause
            .fetch_add(elapsed.as_millis() as u64, Ordering::Relaxed);
    }

    /// Output recorded by the last sequence evaluation (`checkOutput`).
    pub fn last_output(&self) -> String {
        self.last_output.lock().unwrap().clone()
    }

    fn check_deadline(&self) -> Result<(), ScriptFailure> {
        if self.remaining_ms() == 0 {
            return Err(FnError::new(codes::EXECUTION_ABORTED, "Execution timed out.").into());
        }
        Ok(())
    }

    fn count_call(&self, node: &CallNode) -> Result<(), ScriptFailure> {
        self.check_deadline()?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 > self.limits.calls as usize {
            return Err(ScriptError::new(
                codes::EXECUTION_LIMIT,
                "Exceeded the maximum number of function calls.",
                &self.program.source,
                node.offset,
                &self.program.file,
            )
            .into());
        }
        Ok(())
    }

    fn count_output(&self, text: &str) -> Result<(), ScriptFailure> {
        let len = text.chars().count();
        let next = self.output.fetch_add(len, Ordering::Relaxed) + len;
        if next > self.limits.output {
            return Err(
                FnError::new(codes::OUTPUT_LIMIT, "Exceeded the maximum output size.").into(),
            );
        }
        Ok(())
    }

    /// `evaluate(nodes)`: a node's output text.
    pub async fn evaluate(
        &self,
        ctx: &Arc<ScriptContext>,
        node: &Node,
        depth: usize,
    ) -> CallResult {
        match node {
            Node::Text(text) => {
                if ctx.aborted.load(Ordering::Relaxed) {
                    return Err(FnError::new(codes::EXECUTION_ABORTED, "Execution aborted.").into());
                }
                self.count_output(&text.value)?;
                Ok(text.value.clone())
            }
            Node::Call(call) => self.invoke(ctx, call, depth).await,
        }
    }

    /// `evaluate(node[])`: concatenates each child's output.
    pub async fn evaluate_many(
        &self,
        ctx: &Arc<ScriptContext>,
        nodes: &[Node],
        depth: usize,
    ) -> CallResult {
        if nodes.is_empty() {
            return Ok(String::new());
        }
        if nodes.len() == 1 {
            return Box::pin(self.evaluate(ctx, &nodes[0], depth)).await;
        }
        let mut out = String::new();
        for node in nodes {
            let part = Box::pin(self.evaluate(ctx, node, depth)).await?;
            out.push_str(&part);
        }
        *self.last_output.lock().unwrap() = out.clone();
        Ok(out)
    }

    /// `invoke(call)`: resolve, run the builtin, coerce its output.
    pub async fn invoke(
        &self,
        ctx: &Arc<ScriptContext>,
        node: &CallNode,
        depth: usize,
    ) -> CallResult {
        if ctx.aborted.load(Ordering::Relaxed) {
            return Err(FnError::new(codes::EXECUTION_ABORTED, "Execution aborted.").into());
        }
        if depth > self.limits.depth as usize {
            return Err(ScriptError::new(
                codes::EXECUTION_LIMIT,
                "Exceeded the maximum call depth.",
                &self.program.source,
                node.offset,
                &self.program.file,
            )
            .into());
        }
        self.count_call(node)?;
        let definition = {
            let registry = self.registry.read().unwrap();
            registry.resolve(node, &self.program)?
        };
        let call = ScriptCall::new(ctx, self, &node.name, node, depth);
        match (definition.execute)(&call).await {
            Ok(value) => {
                let output = values::text(&value);
                self.count_output(&output)?;
                Ok(output)
            }
            Err(failure) => Err(failure),
        }
    }

    /// `wrapError(body)`: internal kinds (guards, stop, loop control,
    /// partial moderation, execution limits) propagate untouched; anything
    /// else becomes FUNCTION_ERROR.
    pub async fn wrap<F>(&self, body: F) -> CallResult
    where
        F: Future<Output = CallResult>,
    {
        match body.await {
            Err(failure) if !crate::errors::is_uncatchable(&failure) => Err(ScriptFailure::Fn(
                FnError::new(codes::FUNCTION_ERROR, failure.message()),
            )),
            other => other,
        }
    }

    /// `checkOutput(previous)`: if evaluating the call produced output,
    /// replace `previous` with it. Builtins that run sub-scripts pass the
    /// output they captured.
    pub fn check_output(&self, previous: String, current: &str) -> String {
        if current.is_empty() {
            previous
        } else {
            current.to_owned()
        }
    }
}

/// `toText(value)` helper re-export for builtins that need it.
pub fn to_text(value: &Value) -> String {
    values::text(value)
}
