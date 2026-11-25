//! `flush-functions`: Rust port of yuzuha's function-mode scripting
//! language (`$name[arg;arg]` calls inside text).
//!
//! Layout mirrors `yuzuha/src/functions/`:
//! - `ast`, `parser`: the `$`/`[`/`]`/`;`/`\` grammar.
//! - `evaluator`, `call`, `runtime`: lazy-argument evaluation under
//!   per-execution limits.
//! - `registry`, `types`: builtin registration (`defineFunctions`).
//! - `values`: JS-semantic coercions (`text`, `truthy`, `typedScalar`,
//!   `deepEqual`, `toJson`).
//! - `context`: the `ctx` builtins reach (message, services, services,
//!   loop/var state).
//! - `builtins`: the ~459 `$` functions, registered via
//!   [`register_builtins`].

pub mod ast;
pub mod builtins;
pub mod call;
pub mod context;
pub mod errors;
pub mod evaluator;
pub mod limits;
pub mod parser;
pub mod registry;
pub mod runtime;
pub mod types;
pub mod values;

pub use ast::{CallNode, Node, Program, TextNode};
pub use call::{CallResult, ScriptCall};
pub use context::ScriptContext;
pub use errors::{
    codes, FnError, FnResult, GuardError, GuardReason, LoopAction, LoopControl,
    PartialModerationError, ScriptError, ScriptFailure, StopExecution,
};
pub use evaluator::Evaluator;
pub use limits::{Limits, DEFAULT_LIMITS, SOURCE_LIMIT};
pub use parser::parse;
pub use registry::FunctionRegistry;
pub use runtime::Runtime;
pub use types::{define_functions, BoxFnFuture, Definitions, FnOutput, FunctionDefinition};
pub use values::{deep_equal, js_number, js_number_to_string, text, to_json, truthy, typed_scalar};
