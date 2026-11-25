//! Function definition types (port of `runtime/types.ts`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::call::ScriptCall;
use crate::errors::ScriptFailure;

/// Result of a built-in function body.
pub type FnOutput = Result<Value, ScriptFailure>;

/// Boxed future returned by a function body, borrowing the call.
pub type BoxFnFuture<'a> = Pin<Box<dyn Future<Output = FnOutput> + Send + 'a>>;

/// A `$` function the registry can execute.
#[derive(Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub category: String,
    pub min_args: usize,
    pub max_args: usize,
    /// Fully rendered `"$name[usage]"` string.
    pub usage: String,
    pub execute: Arc<dyn for<'a> Fn(&'a ScriptCall<'a>) -> BoxFnFuture<'a> + Send + Sync>,
}

/// Collector mirroring `defineFunctions`.
pub struct Definitions {
    pub list: Vec<FunctionDefinition>,
}

impl Definitions {
    pub fn new() -> Self {
        Self { list: Vec::new() }
    }

    /// `fn(name, category, minArgs, maxArgs, usage, description, execute)`.
    /// `execute` is `|c: &ScriptCall| -> BoxFnFuture`, e.g.
    /// `|c| Box::pin(async move { ... })`.
    #[allow(clippy::too_many_arguments)]
    pub fn add<F>(
        &mut self,
        name: &str,
        category: &str,
        min_args: usize,
        max_args: usize,
        usage: &str,
        description: &str,
        execute: F,
    ) where
        F: for<'a> Fn(&'a ScriptCall<'a>) -> BoxFnFuture<'a> + Send + Sync + 'static,
    {
        self.add_inner(
            name,
            category,
            min_args,
            max_args,
            usage,
            description,
            execute,
            &[],
        );
    }

    /// Same as `add` plus alias names.
    #[allow(clippy::too_many_arguments)]
    pub fn add_aliased<F>(
        &mut self,
        name: &str,
        category: &str,
        min_args: usize,
        max_args: usize,
        usage: &str,
        description: &str,
        execute: F,
        aliases: &[&str],
    ) where
        F: for<'a> Fn(&'a ScriptCall<'a>) -> BoxFnFuture<'a> + Send + Sync + 'static,
    {
        self.add_inner(
            name,
            category,
            min_args,
            max_args,
            usage,
            description,
            execute,
            aliases,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_inner<F>(
        &mut self,
        name: &str,
        category: &str,
        min_args: usize,
        max_args: usize,
        usage: &str,
        description: &str,
        execute: F,
        aliases: &[&str],
    ) where
        F: for<'a> Fn(&'a ScriptCall<'a>) -> BoxFnFuture<'a> + Send + Sync + 'static,
    {
        self.list.push(FunctionDefinition {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            description: description.to_string(),
            category: category.to_string(),
            min_args,
            max_args,
            usage: format!("${name}{usage}"),
            execute: Arc::new(execute),
        });
    }
}

impl Default for Definitions {
    fn default() -> Self {
        Self::new()
    }
}

/// `defineFunctions(setup)`: run `setup` against a fresh collector.
pub fn define_functions(setup: impl FnOnce(&mut Definitions)) -> Vec<FunctionDefinition> {
    let mut d = Definitions::new();
    setup(&mut d);
    d.list
}
