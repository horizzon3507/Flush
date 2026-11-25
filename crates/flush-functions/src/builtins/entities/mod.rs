//! Entity/native builtins (yuzuha `builtins/native-*.ts` and
//! `builtins/entities-*.ts`).

mod channels;
mod collectors;
pub mod common;
mod expressions;
mod info;
mod invites;
mod messages;
mod roles;
mod threads;
mod users;
mod webhooks;

use crate::types::Definitions;

pub fn register(d: &mut Definitions) {
    channels::register(d);
    collectors::register(d);
    expressions::register(d);
    info::register(d);
    invites::register(d);
    messages::register(d);
    roles::register(d);
    threads::register(d);
    users::register(d);
    webhooks::register(d);
}
