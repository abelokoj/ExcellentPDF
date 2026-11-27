//! Signed plugin manifests, capability sandbox, deterministic hook dispatcher,
//! plugin lifecycle registry, and JSON-RPC/stdio runner host (plan E.5).

pub mod hooks;
pub mod manifest;
pub mod runner;
pub mod sandbox;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ed25519_dalek::VerifyingKey;
use serde_json::Value;

use self::hooks::{HookContext, HookDispatchReport, HookDispatcher, HookOutcome};
use self::manifest::{PluginCapability, PluginHook, PluginManifest, verify_signed_plugin};
use self::runner::{PluginHostBridge, PluginRunner};
use self::sandbox::PermissionSandbox;

/// Errors returned by plugin verification, sandboxing, hook dispatch, and execution (plan E.5).
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum PluginError {
    /// Plugin manifest, archive, or digest is invalid.
    InvalidManifest(String),
    /// Ed25519 signature verification failed.
    Signature,
    /// Plugin is not installed in [`PluginRegistry`].
    NotFound(String),
    /// Plugin is currently disabled.
    Disabled(String),
    /// Plugin attempted an operation without the user-granted [`PluginCapability`].
    CapabilityDenied {
        /// Plugin identifier.
        plugin_id: String,
        /// Denied capability.
        capability: PluginCapability,
    },
    /// JSON-RPC runtime or host callback failure.
    Runtime(String),
    /// Underlying I/O error.
    Io(std::io::Error),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(msg) => write!(f, "invalid plugin manifest: {msg}"),
            Self::Signature => f.write_str("plugin signature verification failed"),
            Self::NotFound(id) => write!(f, "plugin not found: {id}"),
            Self::Disabled(id) => write!(f, "plugin is disabled: {id}"),
            Self::CapabilityDenied {
                plugin_id,
                capability,
            } => write!(
                f,
                "plugin {plugin_id} lacks capability {}",
                capability.as_str()
            ),
            Self::Runtime(msg) => write!(f, "plugin runtime error: {msg}"),
            Self::Io(err) => write!(f, "plugin i/o error: {err}"),
        }
    }
}

impl std::error::Error for PluginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidManifest(_)
            | Self::Signature
            | Self::NotFound(_)
            | Self::Disabled(_)
            | Self::CapabilityDenied { .. }
            | Self::Runtime(_) => None,
        }
    }
}

impl From<std::io::Error> for PluginError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Lifecycle state of an installed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PluginLifecycle {
    /// Installed and active for hooks and commands.
    #[default]
    Enabled,
    /// Installed on disk, but hooks and commands are suspended.
    Disabled,
}

/// Installed plugin metadata stored inside [`PluginRegistry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// Verified manifest.
    pub manifest: PluginManifest,
    /// Verified payload bytes.
    pub payload_bytes: Vec<u8>,
    /// Current lifecycle state.
    pub lifecycle: PluginLifecycle,
}

/// Manages signed plugin installation, updates, enable/disable, capability grants, and
/// deterministic hook execution (plan E.5).
#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, InstalledPlugin>,
    sandbox: PermissionSandbox,
    hooks: HookDispatcher,
}

impl PluginRegistry {
    /// Creates an empty [`PluginRegistry`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of installed plugins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether zero plugins are installed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Verifies `archive_bytes` against `verifying_key` and installs (or updates) the plugin,
    /// granting only `approved_capabilities` chosen by the user.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Signature`] or [`PluginError::InvalidManifest`] if verification fails.
    pub fn install_signed(
        &mut self,
        archive_bytes: &[u8],
        verifying_key: &VerifyingKey,
        approved_capabilities: BTreeSet<PluginCapability>,
    ) -> Result<PluginManifest, PluginError> {
        let verified = verify_signed_plugin(archive_bytes, verifying_key)?;
        let id = verified.manifest.id.clone();

        // Only grant capabilities that were both requested and explicitly approved.
        let effective_grants: BTreeSet<PluginCapability> = verified
            .manifest
            .requested_capabilities
            .intersection(&approved_capabilities)
            .copied()
            .collect();
        self.sandbox.set_grants(&id, effective_grants);

        self.hooks.unsubscribe_all(&id);
        for &hook in &verified.manifest.hooks {
            self.hooks.subscribe(&id, hook, verified.manifest.priority);
        }

        let manifest_out = verified.manifest.clone();
        self.plugins.insert(
            id,
            InstalledPlugin {
                manifest: verified.manifest,
                payload_bytes: verified.payload_bytes,
                lifecycle: PluginLifecycle::Enabled,
            },
        );
        Ok(manifest_out)
    }

    /// Enables an installed plugin and re-registers its hooks.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] if `plugin_id` is not installed.
    pub fn enable(&mut self, plugin_id: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))?;
        plugin.lifecycle = PluginLifecycle::Enabled;
        self.hooks.unsubscribe_all(plugin_id);
        for &hook in &plugin.manifest.hooks {
            self.hooks
                .subscribe(plugin_id, hook, plugin.manifest.priority);
        }
        Ok(())
    }

    /// Disables an installed plugin and removes its active hook subscriptions.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] if `plugin_id` is not installed.
    pub fn disable(&mut self, plugin_id: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))?;
        plugin.lifecycle = PluginLifecycle::Disabled;
        self.hooks.unsubscribe_all(plugin_id);
        Ok(())
    }

    /// Uninstalls `plugin_id` and revokes all its capabilities and hooks.
    pub fn uninstall(&mut self, plugin_id: &str) -> Option<InstalledPlugin> {
        self.hooks.unsubscribe_all(plugin_id);
        self.sandbox.clear_plugin(plugin_id);
        self.plugins.remove(plugin_id)
    }

    /// Grants an additional capability to an installed plugin.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] if `plugin_id` is not installed.
    pub fn grant_capability(
        &mut self,
        plugin_id: &str,
        capability: PluginCapability,
    ) -> Result<(), PluginError> {
        if !self.plugins.contains_key(plugin_id) {
            return Err(PluginError::NotFound(plugin_id.to_owned()));
        }
        self.sandbox.grant(plugin_id, capability);
        Ok(())
    }

    /// Revokes `capability` from `plugin_id`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] if `plugin_id` is not installed.
    pub fn revoke_capability(
        &mut self,
        plugin_id: &str,
        capability: PluginCapability,
    ) -> Result<(), PluginError> {
        if !self.plugins.contains_key(plugin_id) {
            return Err(PluginError::NotFound(plugin_id.to_owned()));
        }
        self.sandbox.revoke(plugin_id, capability);
        Ok(())
    }

    /// Looks up an installed plugin record.
    #[must_use]
    pub fn plugin(&self, plugin_id: &str) -> Option<&InstalledPlugin> {
        self.plugins.get(plugin_id)
    }

    /// Read-only view of the [`PermissionSandbox`].
    #[must_use]
    pub const fn sandbox(&self) -> &PermissionSandbox {
        &self.sandbox
    }

    /// Returns subscriber plugin ids for `hook` in deterministic `(priority, plugin_id)` order.
    #[must_use]
    pub fn subscribers_for(&self, hook: PluginHook) -> Vec<String> {
        self.hooks.subscribers_for(hook)
    }

    /// Dispatches `context` across all enabled subscribers in deterministic order.
    ///
    /// # Errors
    ///
    /// Propagates any [`PluginError`] from `handler`.
    pub fn dispatch_hook<F>(
        &self,
        context: HookContext,
        handler: F,
    ) -> Result<HookDispatchReport, PluginError>
    where
        F: FnMut(&str, &HookContext) -> Result<HookOutcome, PluginError>,
    {
        self.hooks.dispatch(context, handler)
    }

    /// Executes `method` on `plugin_id` through `runner` under [`PermissionSandbox`] enforcement.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`], [`PluginError::Disabled`], [`PluginError::CapabilityDenied`],
    /// or [`PluginError::Runtime`].
    pub fn invoke_plugin(
        &self,
        plugin_id: &str,
        method: &str,
        params: &Value,
        runner: &mut dyn PluginRunner,
        host: &mut dyn PluginHostBridge,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .plugins
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))?;
        if matches!(plugin.lifecycle, PluginLifecycle::Disabled) {
            return Err(PluginError::Disabled(plugin_id.to_owned()));
        }
        runner.invoke(plugin_id, method, params, &self.sandbox, host)
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use serde_json::json;

    use super::manifest::{pack_signed_plugin, sha256_hex};
    use super::runner::{HostCallbackRequest, InMemoryJsonRpcPlugin, PluginStep};
    use super::*;

    struct TestHostBridge {
        added_annots: Vec<String>,
    }

    impl PluginHostBridge for TestHostBridge {
        fn required_capability(&self, method: &str) -> Result<PluginCapability, PluginError> {
            match method {
                "doc.info" => Ok(PluginCapability::ReadDocs),
                "annot.add" => Ok(PluginCapability::WriteAnnots),
                "net.fetch" => Ok(PluginCapability::Network),
                "ui.panel" => Ok(PluginCapability::UiPanel),
                other => Err(PluginError::Runtime(format!("unknown host method {other}"))),
            }
        }

        fn call_host(&mut self, method: &str, params: &Value) -> Result<Value, PluginError> {
            match method {
                "doc.info" => Ok(json!({ "pages": 12 })),
                "annot.add" => {
                    let text = params.get("text").and_then(Value::as_str).unwrap_or("note");
                    self.added_annots.push(text.to_owned());
                    Ok(json!({ "ok": true }))
                }
                "net.fetch" => Ok(json!({ "status": 200 })),
                "ui.panel" => Ok(json!({ "rendered": true })),
                other => Err(PluginError::Runtime(format!("unknown host method {other}"))),
            }
        }
    }

    fn make_manifest(
        id: &str,
        caps: &[PluginCapability],
        hooks: &[PluginHook],
        prio: i32,
    ) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: format!("Plugin {id}"),
            version: "1.0.0".to_owned(),
            entry_command: "plugin-bin".to_owned(),
            requested_capabilities: caps.iter().copied().collect(),
            hooks: hooks.iter().copied().collect(),
            priority: prio,
            payload_sha256: sha256_hex(b"sample-plugin-code"),
        }
    }

    #[test]
    fn signed_plugin_executes_when_authorized_and_refuses_missing_capability() {
        let key = SigningKey::from_bytes(&[15_u8; 32]);
        let manifest = make_manifest(
            "auto-cite",
            &[PluginCapability::ReadDocs, PluginCapability::WriteAnnots],
            &[PluginHook::OnDocumentOpen, PluginHook::OnCommand],
            10,
        );
        let archive = pack_signed_plugin(&manifest, b"sample-plugin-code", &key).expect("pack");

        // Tampered package or wrong key is rejected.
        let wrong_key = SigningKey::from_bytes(&[99_u8; 32]).verifying_key();
        let mut reg = PluginRegistry::new();
        assert!(matches!(
            reg.install_signed(&archive, &wrong_key, BTreeSet::new()),
            Err(PluginError::Signature)
        ));

        // Grant ReadDocs ONLY (omit WriteAnnots and Network).
        let grants: BTreeSet<_> = [PluginCapability::ReadDocs].into_iter().collect();
        reg.install_signed(&archive, &key.verifying_key(), grants)
            .expect("install verified plugin");

        let mut host = TestHostBridge {
            added_annots: Vec::new(),
        };

        // 1. Authorized ReadDocs call succeeds.
        let mut read_runner =
            InMemoryJsonRpcPlugin::new(|_method: &str, _params: &Value, cb_res: Option<&Value>| {
                match cb_res {
                    None => Ok(PluginStep::CallHost(HostCallbackRequest {
                        method: "doc.info".to_owned(),
                        params: json!({}),
                    })),
                    Some(info) => Ok(PluginStep::Done(json!({ "seen_pages": info["pages"] }))),
                }
            });
        let res = reg
            .invoke_plugin("auto-cite", "run", &json!({}), &mut read_runner, &mut host)
            .expect("read_docs authorized");
        assert_eq!(res["seen_pages"], 12);

        // 2. Unauthorized WriteAnnots call is refused by sandbox.
        let mut write_runner =
            InMemoryJsonRpcPlugin::new(|_: &str, _: &Value, _: Option<&Value>| {
                Ok(PluginStep::CallHost(HostCallbackRequest {
                    method: "annot.add".to_owned(),
                    params: json!({ "text": "sneaky" }),
                }))
            });
        let denied =
            reg.invoke_plugin("auto-cite", "run", &json!({}), &mut write_runner, &mut host);
        assert!(matches!(
            denied,
            Err(PluginError::CapabilityDenied {
                capability: PluginCapability::WriteAnnots,
                ..
            })
        ));
        assert!(host.added_annots.is_empty());

        // 3. Granting WriteAnnots allows it cleanly.
        reg.grant_capability("auto-cite", PluginCapability::WriteAnnots)
            .expect("grant");
        let mut write_ok_runner =
            InMemoryJsonRpcPlugin::new(|_: &str, _: &Value, cb_res: Option<&Value>| match cb_res {
                None => Ok(PluginStep::CallHost(HostCallbackRequest {
                    method: "annot.add".to_owned(),
                    params: json!({ "text": "verified citation" }),
                })),
                Some(res) => Ok(PluginStep::Done(res.clone())),
            });
        reg.invoke_plugin(
            "auto-cite",
            "run",
            &json!({}),
            &mut write_ok_runner,
            &mut host,
        )
        .expect("write_annots authorized");
        assert_eq!(host.added_annots, vec!["verified citation"]);

        // 4. Disabling plugin blocks invocation and unsubscribes hooks.
        reg.disable("auto-cite").expect("disable");
        assert!(reg.subscribers_for(PluginHook::OnDocumentOpen).is_empty());
        assert!(matches!(
            reg.invoke_plugin("auto-cite", "run", &json!({}), &mut read_runner, &mut host),
            Err(PluginError::Disabled(_))
        ));
    }

    #[test]
    fn deterministic_hook_dispatch_orders_by_priority_then_id() {
        let key = SigningKey::from_bytes(&[21_u8; 32]);
        let mut reg = PluginRegistry::new();
        for (id, prio) in [("zeta", 20), ("alpha", 20), ("first", 5)] {
            let m = make_manifest(
                id,
                &[PluginCapability::ReadDocs],
                &[PluginHook::OnExport],
                prio,
            );
            let pkg = pack_signed_plugin(&m, b"sample-plugin-code", &key).expect("pack");
            reg.install_signed(&pkg, &key.verifying_key(), BTreeSet::new())
                .expect("install");
        }

        let report = reg
            .dispatch_hook(
                HookContext {
                    hook: PluginHook::OnExport,
                    payload: json!({ "step": 0 }),
                },
                |plugin_id, ctx| {
                    let count = ctx.payload["step"].as_u64().unwrap_or(0) + 1;
                    Ok(HookOutcome::Modified(json!({
                        "step": count,
                        "last": plugin_id,
                    })))
                },
            )
            .expect("dispatch");

        assert_eq!(report.executed_plugins, vec!["first", "alpha", "zeta"]);
        assert_eq!(report.final_payload["step"], 3);
        assert_eq!(report.final_payload["last"], "zeta");
    }
}
