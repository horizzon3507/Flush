//! Built-in `$` functions, grouped like yuzuha's `builtins/` tree.

pub mod language;
pub mod values;

use crate::types::Definitions;

/// `registerBuiltins()`: every shipped function in one call.
pub fn register_builtins(d: &mut Definitions) {
    language::register(d);
    values::register(d);
}
