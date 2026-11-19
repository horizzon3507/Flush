//! Message and reaction collectors.
//!
//! The [`Bot`](crate::Bot) feeds every `MESSAGE_CREATE` and
//! `MESSAGE_REACTION_ADD` into its [`Collectors`]. Commands run inline on the
//! event loop, so await a collector from a spawned task
//! (`async_std::task::spawn`), never directly inside `Command::execute`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_std::channel::{unbounded, Receiver, Sender};
use flush_core::{Message, ReactionEmoji, Snowflake};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectedReaction {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    pub user_id: Snowflake,
    pub emoji: ReactionEmoji,
}

type Filter<T> = Box<dyn Fn(&T) -> bool + Send + Sync>;

struct Waiter<T> {
    filter: Filter<T>,
    tx: Sender<T>,
}

struct Registry<T> {
    waiters: Mutex<Vec<Arc<Waiter<T>>>>,
}

impl<T: Clone> Registry<T> {
    fn new() -> Self {
        Self {
            waiters: Mutex::new(Vec::new()),
        }
    }

    fn subscribe(&self, filter: Filter<T>) -> Receiver<T> {
        let (tx, rx) = unbounded();
        let mut waiters = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        waiters.retain(|w| !w.tx.is_closed());
        waiters.push(Arc::new(Waiter { filter, tx }));
        rx
    }

    fn offer(&self, item: &T) {
        // Filters run on a snapshot, without the lock held, so they may use
        // the registry and overlapping offers still reach every waiter.
        let snapshot: Vec<Arc<Waiter<T>>> = {
            let waiters = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
            waiters.clone()
        };
        let dead: Vec<Arc<Waiter<T>>> = snapshot
            .into_iter()
            .filter(|w| {
                if w.tx.is_closed() {
                    return true;
                }
                (w.filter)(item) && w.tx.try_send(item.clone()).is_err()
            })
            .collect();
        if dead.is_empty() {
            return;
        }
        let mut waiters = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        waiters.retain(|w| !dead.iter().any(|d| Arc::ptr_eq(d, w)));
    }

    fn len(&self) -> usize {
        let mut waiters = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        waiters.retain(|w| !w.tx.is_closed());
        waiters.len()
    }
}

/// Stream of collected items. Dropping it unsubscribes.
pub struct Collector<T> {
    rx: Receiver<T>,
}

impl<T> Collector<T> {
    /// Next matching item, or `None` after `timeout`.
    pub async fn next(&self, timeout: Duration) -> Option<T> {
        async_std::future::timeout(timeout, self.rx.recv())
            .await
            .ok()?
            .ok()
    }

    /// Collects up to `max` items within `window`.
    pub async fn collect(&self, max: usize, window: Duration) -> Vec<T> {
        let deadline = std::time::Instant::now() + window;
        let mut out = Vec::new();
        while out.len() < max {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match self.next(left).await {
                Some(item) => out.push(item),
                None => break,
            }
        }
        out
    }
}

/// Shared registry; cheap to clone.
#[derive(Clone)]
pub struct Collectors {
    messages: Arc<Registry<Message>>,
    reactions: Arc<Registry<CollectedReaction>>,
}

impl Default for Collectors {
    fn default() -> Self {
        Self::new()
    }
}

impl Collectors {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Registry::new()),
            reactions: Arc::new(Registry::new()),
        }
    }

    pub fn messages(
        &self,
        filter: impl Fn(&Message) -> bool + Send + Sync + 'static,
    ) -> Collector<Message> {
        Collector {
            rx: self.messages.subscribe(Box::new(filter)),
        }
    }

    /// Messages from `user` in `channel`: the usual "await a reply" case.
    pub fn messages_from(&self, channel: Snowflake, user: Snowflake) -> Collector<Message> {
        self.messages(move |m| m.channel_id == channel && m.author.id == user)
    }

    pub fn reactions(
        &self,
        filter: impl Fn(&CollectedReaction) -> bool + Send + Sync + 'static,
    ) -> Collector<CollectedReaction> {
        Collector {
            rx: self.reactions.subscribe(Box::new(filter)),
        }
    }

    pub fn reactions_on(&self, message: Snowflake) -> Collector<CollectedReaction> {
        self.reactions(move |r| r.message_id == message)
    }

    pub fn offer_message(&self, message: &Message) {
        self.messages.offer(message);
    }

    pub fn offer_reaction(&self, reaction: &CollectedReaction) {
        self.reactions.offer(reaction);
    }

    /// Live subscriptions (messages + reactions).
    pub fn active(&self) -> usize {
        self.messages.len() + self.reactions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reaction(message: u64, user: u64) -> CollectedReaction {
        CollectedReaction {
            channel_id: Snowflake(1),
            message_id: Snowflake(message),
            user_id: Snowflake(user),
            emoji: serde_json::from_value(serde_json::json!({ "name": "👍" })).unwrap(),
        }
    }

    #[async_std::test]
    async fn filters_and_times_out() {
        let collectors = Collectors::new();
        let on_five = collectors.reactions_on(Snowflake(5));
        collectors.offer_reaction(&reaction(4, 1));
        collectors.offer_reaction(&reaction(5, 2));
        collectors.offer_reaction(&reaction(5, 3));
        let got = on_five.collect(5, Duration::from_millis(30)).await;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].user_id, Snowflake(2));
        assert!(on_five.next(Duration::from_millis(5)).await.is_none());
    }

    #[async_std::test]
    async fn filter_may_use_the_registry() {
        let collectors = Collectors::new();
        let inner = collectors.clone();
        let c = collectors.reactions(move |_| {
            let _ = inner.active();
            true
        });
        collectors.offer_reaction(&reaction(5, 1));
        assert!(c.next(Duration::from_millis(20)).await.is_some());
    }

    #[async_std::test]
    async fn subscribing_prunes_dropped_collectors() {
        let collectors = Collectors::new();
        drop(collectors.reactions_on(Snowflake(5)));
        let _live = collectors.reactions_on(Snowflake(6));
        let registered = collectors
            .reactions
            .waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        assert_eq!(registered, 1);
    }

    #[async_std::test]
    async fn offer_inside_a_filter_still_reaches_other_collectors() {
        let collectors = Collectors::new();
        let inner = collectors.clone();
        // Stands in for an overlapping offer: it runs while the first is mid-flight.
        let _nested = collectors.reactions(move |r| {
            if r.message_id == Snowflake(5) {
                inner.offer_reaction(&reaction(6, 9));
            }
            false
        });
        let on_six = collectors.reactions_on(Snowflake(6));
        collectors.offer_reaction(&reaction(5, 1));
        let got = on_six.next(Duration::from_millis(20)).await;
        assert_eq!(got.map(|r| r.user_id), Some(Snowflake(9)));
    }

    #[async_std::test]
    async fn dropped_collector_unsubscribes() {
        let collectors = Collectors::new();
        let c = collectors.reactions_on(Snowflake(5));
        assert_eq!(collectors.active(), 1);
        drop(c);
        collectors.offer_reaction(&reaction(5, 1));
        assert_eq!(collectors.active(), 0);
    }
}
