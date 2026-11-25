//! Function registry (port of `runtime/registry.ts`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ast::{CallNode, Node, Program};
use crate::errors::{FnError, ScriptError};
use crate::types::FunctionDefinition;

#[derive(Default)]
pub struct FunctionRegistry {
    entries: HashMap<String, Arc<FunctionDefinition>>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one definition under its name and aliases (lowercased).
    pub fn register(&mut self, definition: FunctionDefinition) -> Result<(), FnError> {
        if definition.min_args > definition.max_args {
            return Err(FnError::new("FUNCTION", "Invalid arity."));
        }
        let mut names = vec![definition.name.clone()];
        names.extend(definition.aliases.iter().cloned());
        let normalized: Vec<String> = names
            .iter()
            .map(|n| n.trim_start_matches('$').to_lowercase())
            .collect();
        if normalized.iter().collect::<HashSet<_>>().len() != normalized.len() {
            return Err(FnError::new("FUNCTION", "Duplicate alias."));
        }
        for name in &normalized {
            if !valid_name(name) || self.entries.contains_key(name) {
                return Err(FnError::new(
                    "FUNCTION",
                    format!("Invalid or duplicate function: ${name}."),
                ));
            }
        }
        let shared = Arc::new(definition);
        for name in normalized {
            self.entries.insert(name, shared.clone());
        }
        Ok(())
    }

    /// Register many atomically: any failure leaves the registry unchanged.
    pub fn register_all(&mut self, definitions: Vec<FunctionDefinition>) -> Result<(), FnError> {
        let mut staged = FunctionRegistry::new();
        for definition in self
            .functions()
            .into_iter()
            .map(|f| (*f).clone())
            .chain(definitions)
        {
            staged.register(definition)?;
        }
        self.entries = staged.entries;
        Ok(())
    }

    /// Remove every name (and its aliases) from the registry.
    pub fn unregister_all(&mut self, names: &[String]) {
        let mut targets: HashSet<*const FunctionDefinition> = HashSet::new();
        for raw in names {
            let normalized = raw.trim_start_matches('$').to_lowercase();
            if let Some(f) = self.entries.get(&normalized) {
                targets.insert(Arc::as_ptr(f));
            }
        }
        self.entries
            .retain(|_, f| !targets.contains(&Arc::as_ptr(f)));
    }

    /// One entry per registered definition (aliases deduplicated).
    pub fn functions(&self) -> Vec<Arc<FunctionDefinition>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for f in self.entries.values() {
            if seen.insert(Arc::as_ptr(f)) {
                out.push(f.clone());
            }
        }
        out
    }

    /// Resolve a call node to its definition (name + arity check).
    pub fn resolve(
        &self,
        node: &CallNode,
        program: &Program,
    ) -> Result<Arc<FunctionDefinition>, ScriptError> {
        let name = node.name.to_lowercase();
        let f = self.entries.get(&name).ok_or_else(|| {
            ScriptError::new(
                "UNKNOWN_FUNCTION",
                format!("Unknown function: ${}.", node.name),
                &program.source,
                node.offset,
                &program.file,
            )
        })?;
        if node.args.len() < f.min_args || node.args.len() > f.max_args {
            return Err(ScriptError::new(
                "ARITY",
                format!("Usage: {}", f.usage),
                &program.source,
                node.offset,
                &program.file,
            ));
        }
        Ok(f.clone())
    }

    /// Resolve every call in a compiled program (compile-time validation).
    pub fn validate(&self, program: &Program) -> Result<(), ScriptError> {
        fn visit(
            registry: &FunctionRegistry,
            nodes: &[Node],
            program: &Program,
        ) -> Result<(), ScriptError> {
            for node in nodes {
                if let Node::Call(call) = node {
                    registry.resolve(call, program)?;
                    for arg in &call.args {
                        visit(registry, arg, program)?;
                    }
                }
            }
            Ok(())
        }
        visit(self, &program.nodes, program)
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}
