//! Replayable session-event publishing infrastructure.
//!
//! This module owns the canonical [`AgentSessionContext`] path for publishing
//! UI-replayable session events. Every replayable event must go through an
//! atomic publisher — this helper, the runtime child-event publisher, or an
//! event forwarder — so that:
//!
//! 1. The event is cached on the session's runner for late-subscriber replay
//!    *before* any subscriber sees it on the live broadcast channel.
//! 2. The cache-then-broadcast ordering is uniform across all writers, so
//!    reconnecting clients can never miss an event that earlier subscribers
//!    received.
//!
//! ## Invariant
//!
//! No code in the workspace may pair `runner.push_critical_event` with
//! `sender.send` outside this module, the runtime
//! `ReplayableSessionEventPublisher`, the generic engine event forwarder, or
//! the server's `spawn_event_forwarder`
//! (in `handlers::agent::execute::runtime::events`).
//! Hand-rolling the pair has historically led to inverted ordering (broadcast
//! first, cache second), which leaves a small window where a late subscriber
//! receives the live event but the cache is still empty.
//!
//! ## When to use which helper
//!
//! Use [`publish_replayable_session_event`] for **synchronous** writers that
//! hold an [`AgentSessionContext`] directly — e.g. HTTP handlers
//! (`patch_session`, `regenerate_title`) and background tasks (`title_gen`).
//! Use [`publish_replayable_session_event_via_parts`] when the writer holds
//! the raw registries instead (e.g. the connect bridge's `ConnectContext`) —
//! the ordering lives here either way. The runtime forwarder
//! (`spawn_event_forwarder`) handles the equivalent ordering for events that
//! flow through the engine's mpsc channel.

use std::collections::HashMap;
use std::sync::Arc;

use bamboo_agent_core::AgentEvent;
use tokio::sync::{broadcast, RwLock as TokioRwLock};

use crate::app_context::AgentSessionContext;
use crate::runtime::execution::event_forwarder::AccountFeedInbox;
use crate::AgentRunner;

/// Publishes a replayable session event with the correct cache-then-broadcast
/// ordering. See module docs for the invariant.
///
/// Order is fixed and **must not** be changed:
///
/// 1. `runner.push_critical_event(event.clone())` — populate the late-subscriber
///    replay cache while the event is still un-broadcast.
/// 2. `sender.send(event)` — broadcast to all live subscribers before releasing
///    the runner lock that guards subscription snapshots.
///
/// If no runner exists for `session_id` (the session has not started yet, or
/// has already terminated), the cache step is silently skipped and the event
/// is still broadcast. This matches the semantics of the runtime forwarder.
pub async fn publish_replayable_session_event(
    ctx: &dyn AgentSessionContext,
    session_id: &str,
    event: AgentEvent,
) {
    // Resolve the sender first so publishers and subscribers never invert the
    // session-event and runner lock order.
    let sender = ctx.get_session_event_sender(session_id).await;
    {
        let mut runners = ctx.agent_runners().write().await;
        if let Some(runner) = runners.get_mut(session_id) {
            runner.push_critical_event(event.clone());
        }

        // Mirror onto the account-wide change feed (sequenced + journaled).
        // The sink filters ephemeral events internally; metadata events
        // published here (title/pinned) are durable and will be sequenced.
        ctx.account_sink().record(Some(session_id), &event);

        // Keep cache mutation and live publication inside one runner-lock
        // boundary. Subscribers clone the cache while holding the matching
        // read lock, so an event can appear in either their snapshot or their
        // receiver, never both.
        let _ = sender.send(event);
    }
}

/// Parts-based variant of [`publish_replayable_session_event`] for writers
/// that hold the raw registries instead of an [`AgentSessionContext`] (the
/// connect bridge publishes chat-channel switches this way). The
/// cache-then-broadcast ordering is identical — this module remains the only
/// place that pairs `runner.push_critical_event` with `sender.send`.
///
/// `sender` is the caller-resolved broadcast channel for `session_id`
/// (get-or-create semantics live with the caller); pass `None` only when no
/// channel can exist — the runner replay cache and account feed still get the
/// event, only the live broadcast is skipped. `account_feed_inbox` mirrors the
/// event onto the account-wide change feed.
pub async fn publish_replayable_session_event_via_parts(
    runners: &Arc<TokioRwLock<HashMap<String, AgentRunner>>>,
    sender: Option<&broadcast::Sender<AgentEvent>>,
    account_feed_inbox: Option<&AccountFeedInbox>,
    session_id: &str,
    event: AgentEvent,
) {
    {
        let mut runners = runners.write().await;
        if let Some(runner) = runners.get_mut(session_id) {
            runner.push_critical_event(event.clone());
        }

        if let Some(inbox) = account_feed_inbox {
            let _ = inbox
                .send((Some(session_id.to_string()), event.clone()))
                .await;
        }

        if let Some(sender) = sender {
            let _ = sender.send(event);
        }
    }
}
