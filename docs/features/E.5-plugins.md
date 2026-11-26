# Ph9: 13/13 | E.5: Plugins

**Tag:** E.5; **Priority:** P2; **Phase:** 9; **Needs:** E.2, K.21; **Code:** `excellent-features::plugins`
**Plan:** §27 entry `#### Ph9: 13/13 | E.5`.

## Problem

Power users and institutions need third-party extensions without exposing documents, annotations, or network access to untrusted unsigned code.

## User story

As a user, I want to install Ed25519-signed plugins, grant or revoke individual capabilities (`ReadDocs`, `WriteAnnots`, `Network`, `UiPanel`), and have unauthorized tool calls refused automatically.

## UI sketch

- Settings -> Plugins manager listing installed plugins, signature status, enable/disable toggle, and per-capability permission checkboxes.

## Mode

All modes.

## Speed and size impact

- Startup: enabled plugin manifests load lazily; runner processes spawn only on hook or command invocation.
- Budgets touched: O(1) capability authorization per host callback.
- Installer size: 0 bytes added (pure Rust + stdio JSON-RPC 2.0 host).

## Design

- Model: `excellent-features::plugins::{PluginRegistry, PluginLifecycle}` + `plugins::manifest::{PluginManifest, PluginCapability, PluginHook, pack_signed_plugin, verify_signed_plugin}` + `plugins::sandbox::PermissionSandbox` + `plugins::hooks::HookDispatcher` + `plugins::runner::{PluginRunner, InMemoryJsonRpcPlugin, StdioJsonRpcRunner}`.

## Acceptance tests

- [ ] `signed_plugin_executes_when_authorized_and_refuses_missing_capability`: Verifies Ed25519 signature enforcement, executes authorized `ReadDocs` callback, and refuses ungranted `WriteAnnots` callback with `PluginError::CapabilityDenied`.
- [ ] `deterministic_hook_dispatch_orders_by_priority_then_id`: Executes hook subscribers in `(priority, plugin_id)` order.
