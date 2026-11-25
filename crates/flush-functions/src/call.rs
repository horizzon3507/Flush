//! `ScriptCall`: the first argument of every `$` function (mirrors
//! yuzuha's evaluator `call` object: `arg`, `fresh`, `text`, `all`,
//! `checkOutput`, `wait`).

use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::ast::CallNode;
use crate::context::ScriptContext;
use crate::errors::ScriptFailure;
use crate::evaluator::Evaluator;

pub type CallResult = Result<String, ScriptFailure>;

/// One `$name[...]` invocation. Argument nodes stay unevaluated until the
/// function asks for them (`arg` caches per index; `fresh` re-evaluates).
pub struct ScriptCall<'a> {
    pub ctx: &'a Arc<ScriptContext>,
    pub evaluator: &'a Evaluator,
    pub name: &'a str,
    pub node: &'a CallNode,
    pub depth: usize,
    cache: Vec<Mutex<Option<String>>>,
}

impl<'a> ScriptCall<'a> {
    pub fn new(
        ctx: &'a Arc<ScriptContext>,
        evaluator: &'a Evaluator,
        name: &'a str,
        node: &'a CallNode,
        depth: usize,
    ) -> Self {
        Self {
            ctx,
            evaluator,
            name,
            node,
            depth,
            cache: node.args.iter().map(|_| Mutex::new(None)).collect(),
        }
    }

    pub fn arg_count(&self) -> usize {
        self.node.args.len()
    }

    /// `arg(i)`: lazy, cached evaluation of argument `i`.
    pub async fn arg(&self, index: usize) -> CallResult {
        if index >= self.node.args.len() {
            return Ok(String::new());
        }
        if let Some(hit) = self.cache[index].lock().unwrap().clone() {
            return Ok(hit);
        }
        let value = Box::pin(self.evaluator.evaluate_many(
            self.ctx,
            &self.node.args[index],
            self.depth + 1,
        ))
        .await?;
        *self.cache[index].lock().unwrap() = Some(value.clone());
        Ok(value)
    }

    /// `fresh(i)`: re-evaluate argument `i`, ignoring the cache.
    pub async fn fresh(&self, index: usize) -> CallResult {
        if index >= self.node.args.len() {
            return Ok(String::new());
        }
        Box::pin(
            self.evaluator
                .evaluate_many(self.ctx, &self.node.args[index], self.depth + 1),
        )
        .await
    }

    /// `text(i, separator = " ")`: all arguments from `i` joined.
    pub async fn text(&self, index: usize, separator: Option<&str>) -> CallResult {
        let sep = separator.unwrap_or(" ");
        let mut parts = Vec::new();
        for i in index..self.node.args.len() {
            parts.push(Box::pin(self.arg(i)).await?);
        }
        Ok(parts.join(sep))
    }

    /// `all(separator = " ")`: `text(0, separator)`.
    pub async fn all(&self, separator: Option<&str>) -> CallResult {
        Box::pin(self.text(0, separator)).await
    }

    /// `checkOutput(previous, current)`: prefer `current` when the nested
    /// run produced output.
    pub fn check_output(&self, previous: String, current: &str) -> String {
        self.evaluator.check_output(previous, current)
    }

    /// `wait(future)`: pause the deadline while awaiting outside work.
    pub async fn wait<F, T>(&self, future: F) -> Result<T, ScriptFailure>
    where
        F: Future<Output = Result<T, ScriptFailure>> + Send,
    {
        let started = std::time::Instant::now();
        let result = future.await;
        self.evaluator.wait_started(started.elapsed());
        result
    }
}
