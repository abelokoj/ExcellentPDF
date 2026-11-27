//! Permission sandbox enforcing least-privilege plugin capabilities (plan E.5).

use std::collections::{BTreeMap, BTreeSet};

use ecclesia_core::tools::ToolCapability;

use super::PluginError;
use super::manifest::PluginCapability;

/// Enforces user-granted [`PluginCapability`] permissions per installed plugin (plan E.5).
#[derive(Debug, Clone, Default)]
pub struct PermissionSandbox {
    granted: BTreeMap<String, BTreeSet<PluginCapability>>,
}

impl PermissionSandbox {
    /// Creates an empty sandbox with zero granted permissions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a plugin with an initial set of granted capabilities.
    pub fn set_grants(&mut self, plugin_id: &str, caps: BTreeSet<PluginCapability>) {
        self.granted.insert(plugin_id.to_owned(), caps);
    }

    /// Grants a single `capability` to `plugin_id`.
    pub fn grant(&mut self, plugin_id: &str, capability: PluginCapability) {
        self.granted
            .entry(plugin_id.to_owned())
            .or_default()
            .insert(capability);
    }

    /// Revokes `capability` from `plugin_id`.
    pub fn revoke(&mut self, plugin_id: &str, capability: PluginCapability) {
        if let Some(set) = self.granted.get_mut(plugin_id) {
            set.remove(&capability);
        }
    }

    /// Removes all granted capabilities for `plugin_id`.
    pub fn clear_plugin(&mut self, plugin_id: &str) {
        self.granted.remove(plugin_id);
    }

    /// Returns the currently granted capabilities for `plugin_id`.
    #[must_use]
    pub fn grants_for(&self, plugin_id: &str) -> BTreeSet<PluginCapability> {
        self.granted.get(plugin_id).cloned().unwrap_or_default()
    }

    /// Authorizes `plugin_id` to perform an operation requiring `capability`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::CapabilityDenied`] if `capability` has not been granted.
    pub fn authorize(
        &self,
        plugin_id: &str,
        capability: PluginCapability,
    ) -> Result<(), PluginError> {
        let allowed = self
            .granted
            .get(plugin_id)
            .is_some_and(|set| set.contains(&capability));
        if !allowed {
            return Err(PluginError::CapabilityDenied {
                plugin_id: plugin_id.to_owned(),
                capability,
            });
        }
        Ok(())
    }

    /// Maps an `ecclesia-core` [`ToolCapability`] to the required [`PluginCapability`] and checks
    /// authorization for `plugin_id`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::CapabilityDenied`] if the mapped capability is not granted.
    pub fn authorize_tool(
        &self,
        plugin_id: &str,
        tool_capability: ToolCapability,
    ) -> Result<(), PluginError> {
        let mapped = map_tool_capability(tool_capability);
        self.authorize(plugin_id, mapped)
    }
}

/// Maps a core tool capability classification to the corresponding [`PluginCapability`].
#[must_use]
pub const fn map_tool_capability(cap: ToolCapability) -> PluginCapability {
    match cap {
        ToolCapability::ReadOnly => PluginCapability::ReadDocs,
        ToolCapability::MutateDocument | ToolCapability::FileSystemWrite => {
            PluginCapability::WriteAnnots
        }
        ToolCapability::UiControl => PluginCapability::UiPanel,
    }
}
