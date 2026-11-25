//! Language builtins (yuzuha `builtins/language/*`).

pub mod array_callbacks;
pub mod arrays;
pub mod common;
pub mod decimal;
pub mod decimals;
pub mod locals;
pub mod loops;
pub mod numbers;
pub mod objects;
pub mod regex;
pub mod sort;
pub mod state;
pub mod texts;

use crate::types::Definitions;

pub fn register(d: &mut Definitions) {
    array_callbacks::register(d);
    arrays::register(d);
    decimals::register(d);
    locals::register(d);
    loops::register(d);
    numbers::register(d);
    objects::register(d);
    regex::register(d);
    sort::register(d);
    state::register(d);
    texts::register(d);
}
