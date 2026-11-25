//! Script builtins for services: context, guards, flow, storage, events,
//! schedule, http, moderation and the pure SDK helpers.

mod canvas;
mod context;
mod events;
mod flow;
mod guards;
mod http;
mod moderation;
mod music;
mod pure_cdn;
mod pure_collections;
mod pure_errors;
mod pure_flags;
mod pure_utils;
mod schedule;
mod sdk_data;
mod storage;
mod support;
mod voice;

pub fn register(d: &mut crate::types::Definitions) {
    context::register(d);
    guards::register(d);
    flow::register(d);
    storage::register(d);
    events::register(d);
    schedule::register(d);
    http::register(d);
    moderation::register(d);
    pure_utils::register(d);
    pure_flags::register(d);
    pure_errors::register(d);
    pure_collections::register(d);
    pure_cdn::register(d);
    voice::register(d);
    music::register(d);
    canvas::register(d);
}
