//! Deterministic plugin hook dispatcher ordered by `(priority, plugin_id)` (plan E.5).

use std::collections::BTreeSet;

use serde_json::Value;

use super::PluginError;
use super::manifest::PluginHook;

/// Context payload passed into a plugin hook invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct HookContext {
    /// Hook event being dispatched.
    pub hook: PluginHook,
    /// Structured JSON payload for the event.
    pub payload: Value,
}

/// Outcome returned by an individual plugin hook handler.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HookOutcome {
    /// Proceed unchanged to the next subscriber.
    Continue,
    /// Replace the event payload for downstream subscribers.
    Modified(Value),
    /// Abort further hook propagation with a reason.
    Cancel(String),
}

/// Deterministic hook subscriber entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HookSubscription {
    /// Lower numeric priority executes earlier.
    pub priority: i32,
    /// Unique plugin id used as deterministic tie-breaker.
    pub plugin_id: String,
    /// Subscribed hook kind.
    pub hook: PluginHook,
}

/// Aggregated result from dispatching a hook across all enabled subscribers.
#[derive(Debug, Clone, PartialEq)]
pub struct HookDispatchReport {
    /// Final payload after any [`HookOutcome::Modified`] transforms.
    pub final_payload: Value,
    /// Ordered list of plugin ids executed.
    pub executed_plugins: Vec<String>,
    /// Cancellation reason if any plugin returned [`HookOutcome::Cancel`].
    pub cancelled_by: Option<(String, String)>,
}

/// Maintains deterministic hook subscriptions for installed plugins (plan E.5).
#[derive(Debug, Clone, Default)]
pub struct HookDispatcher {
    subscriptions: BTreeSet<HookSubscription>,
}

impl HookDispatcher {
    /// Creates an empty hook dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `plugin_id` for `hook` at `priority`.
    pub fn subscribe(&mut self, plugin_id: &str, hook: PluginHook, priority: i32) {
        self.subscriptions.insert(HookSubscription {
            priority,
            plugin_id: plugin_id.to_owned(),
            hook,
        });
    }

    /// Unregisters all hooks belonging to `plugin_id`.
    pub fn unsubscribe_all(&mut self, plugin_id: &str) {
        self.subscriptions.retain(|s| s.plugin_id != plugin_id);
    }

    /// Returns subscriber plugin ids for `hook` in deterministic `(priority, plugin_id)` order.
    #[must_use]
    pub fn subscribers_for(&self, hook: PluginHook) -> Vec<String> {
        self.subscriptions
            .iter()
            .filter(|s| s.hook == hook)
            .map(|s| s.plugin_id.clone())
            .collect()
    }

    /// Dispatches `context` across subscribed enabled plugins using `handler`.
    ///
    /// # Errors
    ///
    /// Propagates any [`PluginError`] returned by `handler`.
    pub fn dispatch<F>(
        &self,
        mut context: HookContext,
        mut handler: F,
    ) -> Result<HookDispatchReport, PluginError>
    where
        F: FnMut(&str, &HookContext) -> Result<HookOutcome, PluginError>,
    {
        let subs = self.subscribers_for(context.hook);
        let mut executed_plugins = Vec::new();

        for plugin_id in subs {
            let outcome = handler(&plugin_id, &context)?;
            executed_plugins.push(plugin_id.clone());
            match outcome {
                HookOutcome::Continue => {}
                HookOutcome::Modified(next_payload) => {
                    context.payload = next_payload;
                }
                HookOutcome::Cancel(reason) => {
                    return Ok(HookDispatchReport {
                        final_payload: context.payload,
                        executed_plugins,
                        cancelled_by: Some((plugin_id, reason)),
                    });
                }
            }
        }

        Ok(HookDispatchReport {
            final_payload: context.payload,
            executed_plugins,
            cancelled_by: None,
        })
    }
}
