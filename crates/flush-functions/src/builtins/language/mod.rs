//! Language builtins (yuzuha `builtins/language/*`).

pub mod locals;

use crate::types::Definitions;

pub fn register(d: &mut Definitions) {
    locals::register(d);
}
