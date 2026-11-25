//! `Runtime`: compiles source once and evaluates it against a
//! `ScriptContext` (mirrors yuzuha's `Runtime` — the `ScriptEngine`
//! port implementation).

use std::sync::{Arc, RwLock};

use crate::ast::Program;
use crate::context::ScriptContext;
use crate::errors::{codes, FnError, ScriptError, ScriptFailure};
use crate::evaluator::Evaluator;
use crate::limits::{Limits, DEFAULT_LIMITS, SOURCE_LIMIT};
use crate::parser::parse;
use crate::registry::FunctionRegistry;
use crate::types::FunctionDefinition;

pub struct Runtime {
    limits: Limits,
    registry: Arc<RwLock<FunctionRegistry>>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_LIMITS)
    }

    /// `new Runtime(limits)`.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
            registry: Arc::new(RwLock::new(FunctionRegistry::new())),
        }
    }

    /// `fork()`: shares the registry; recompiles per run.
    pub fn fork(&self) -> Self {
        Self {
            limits: self.limits,
            registry: self.registry.clone(),
        }
    }

    /// `compile(source)` — parse once; rejects over the source limit.
    pub fn compile(&self, source: &str) -> Result<Arc<Program>, ScriptFailure> {
        self.compile_file(source, "")
    }

    /// `compile(source, file)` — parse with a file label for errors.
    pub fn compile_file(&self, source: &str, file: &str) -> Result<Arc<Program>, ScriptFailure> {
        if source.chars().count() > SOURCE_LIMIT {
            return Err(FnError::new(codes::SOURCE_LIMIT, "Source too large.").into());
        }
        let program = parse(source, file).map_err(ScriptFailure::Script)?;
        Ok(Arc::new(program))
    }

    /// `execute(ctx, program)` — evaluate a compiled program.
    pub async fn execute(
        &self,
        ctx: &Arc<ScriptContext>,
        program: &Arc<Program>,
    ) -> Result<String, ScriptFailure> {
        let evaluator = Evaluator::new(program.clone(), self.limits, self.registry.clone());
        evaluator.evaluate_many(ctx, &program.nodes, 0).await
    }

    /// `execute(ctx, source)` — compile + run in one call.
    pub async fn run(
        &self,
        ctx: &Arc<ScriptContext>,
        source: &str,
    ) -> Result<String, ScriptFailure> {
        let program = self.compile(source)?;
        self.execute(ctx, &program).await
    }

    /// `register(definition)`.
    pub fn register(&self, definition: FunctionDefinition) -> Result<(), FnError> {
        self.registry.write().unwrap().register(definition)
    }

    /// `registerAll(definitions)` — stage + swap, atomic.
    pub fn register_all(&self, definitions: Vec<FunctionDefinition>) -> Result<(), FnError> {
        self.registry.write().unwrap().register_all(definitions)
    }

    /// `unregisterAll(names)` — remove the given names and their aliases.
    pub fn unregister_all(&self, names: &[String]) {
        self.registry.write().unwrap().unregister_all(names);
    }

    /// `functions()` — every registered definition.
    pub fn functions(&self) -> Vec<Arc<FunctionDefinition>> {
        self.registry.read().unwrap().functions()
    }

    /// `validate(program)` — resolve every call at compile time.
    pub fn validate(&self, program: &Program) -> Result<(), ScriptError> {
        self.registry.read().unwrap().validate(program)
    }

    /// `text(ctx)` — the program source (port parity stub).
    pub fn text<'a>(&self, program: &'a Program) -> &'a str {
        &program.source
    }

    pub fn registry(&self) -> &Arc<RwLock<FunctionRegistry>> {
        &self.registry
    }
}
