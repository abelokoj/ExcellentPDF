//! Tool table registry, path confinement, and JSON-Schema-validated dispatch (plan E.2).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::registry::Registry;

use super::builtin::register_builtin_tools;
use super::schema::validate_json;

/// Coarse capability category for a registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCapability {
    /// Reads documents without modifying files on disk.
    ReadOnly,
    /// Mutates PDF documents (annotations, saves).
    MutateDocument,
    /// Writes new output files to disk.
    FileSystemWrite,
    /// Drives interactive UI state.
    UiControl,
}

/// Metadata and JSON Schemas describing one tool in the Agent Tool Table.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Dotted tool identifier (e.g. `"doc.info"`, `"annot.list"`).
    pub name: String,
    /// Concise human/agent description of what the tool does.
    pub description: String,
    /// JSON Schema describing valid input arguments.
    pub input_schema: Value,
    /// JSON Schema describing the structured output object.
    pub output_schema: Value,
    /// Valid example arguments matching [`Self::input_schema`].
    pub example_input: Value,
    /// Required capability classification.
    pub capability: ToolCapability,
}

/// Errors produced while resolving paths, validating inputs, or executing a tool.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum ToolError {
    /// Requested tool name is not registered.
    UnknownTool(String),
    /// Arguments failed JSON Schema validation or semantic checks.
    InvalidArguments(String),
    /// A path fell outside the configured `--root` confinement directory.
    PathDenied {
        /// The requested path.
        path: PathBuf,
        /// The allowed root directory.
        root: PathBuf,
    },
    /// Document-layer error.
    Document(crate::document::Error),
    /// MuPDF-layer error.
    Mupdf(ecclesia_mupdf::MuError),
    /// Filesystem or transport I/O error.
    Io(std::io::Error),
    /// General tool execution failure.
    Execution(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidArguments(msg) => write!(f, "invalid tool arguments: {msg}"),
            Self::PathDenied { path, root } => write!(
                f,
                "access denied: {} is outside allowed root {}",
                path.display(),
                root.display()
            ),
            Self::Document(err) => write!(f, "{err}"),
            Self::Mupdf(err) => write!(f, "{err}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::Execution(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Document(err) => Some(err),
            Self::Mupdf(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::UnknownTool(_)
            | Self::InvalidArguments(_)
            | Self::PathDenied { .. }
            | Self::Execution(_) => None,
        }
    }
}

impl From<crate::document::Error> for ToolError {
    fn from(err: crate::document::Error) -> Self {
        Self::Document(err)
    }
}

impl From<ecclesia_mupdf::MuError> for ToolError {
    fn from(err: ecclesia_mupdf::MuError) -> Self {
        Self::Mupdf(err)
    }
}

impl From<std::io::Error> for ToolError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Policy controlling whether [`ToolRegistry::execute_script`] stops on the first failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScriptFailurePolicy {
    /// Abort remaining script steps after the first failed tool call.
    #[default]
    StopOnError,
    /// Execute every step in the script even if an earlier step fails.
    Continue,
}

/// Execution context passed to tool handlers, including shared document registry and optional root jail.
#[derive(Debug, Clone)]
pub struct ToolContext {
    registry: Arc<Registry>,
    /// Optional root folder confining all file reads and writes (for `ecclesia-cli mcp --root`).
    pub root_dir: Option<PathBuf>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolContext {
    /// Creates an unconfined context backed by a fresh document registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Registry::new()),
            root_dir: None,
        }
    }

    /// Creates a context whose path operations are confined under `root`.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Io`] if `root` does not exist or cannot be canonicalized, or
    /// [`ToolError::InvalidArguments`] if `root` is not a directory.
    pub fn with_root(root: impl AsRef<Path>) -> Result<Self, ToolError> {
        let canonical = std::fs::canonicalize(root.as_ref())?;
        if !canonical.is_dir() {
            return Err(ToolError::InvalidArguments(format!(
                "root path is not a directory: {}",
                canonical.display()
            )));
        }
        Ok(Self {
            registry: Arc::new(Registry::new()),
            root_dir: Some(canonical),
        })
    }

    /// Shared open-document registry for this context.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Resolves `raw_path`, enforcing `root_dir` confinement if configured.
    ///
    /// Handles paths whose final filename does not exist yet (e.g., output PDFs) by canonicalizing
    /// the deepest existing ancestor directory and normalizing remaining components.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::PathDenied`] if the resolved path escapes `root_dir`, or
    /// [`ToolError::InvalidArguments`] for empty or malformed paths.
    pub fn resolve_path(&self, raw_path: &str) -> Result<PathBuf, ToolError> {
        if raw_path.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "path must not be empty".to_owned(),
            ));
        }
        let candidate = Path::new(raw_path);
        let joined = match &self.root_dir {
            Some(root) if candidate.is_relative() => root.join(candidate),
            _ => candidate.to_path_buf(),
        };
        let resolved = normalize_with_existing_ancestor(&joined)?;
        if let Some(root) = &self.root_dir
            && !resolved.starts_with(root)
        {
            return Err(ToolError::PathDenied {
                path: resolved,
                root: root.clone(),
            });
        }
        Ok(resolved)
    }
}

fn normalize_with_existing_ancestor(path: &Path) -> Result<PathBuf, ToolError> {
    if path.exists() {
        return Ok(std::fs::canonicalize(path)?);
    }
    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(ToOwned::to_owned) else {
            break;
        };
        tail.push(name);
        if !existing.pop() {
            break;
        }
    }
    let base = if existing.exists() {
        std::fs::canonicalize(&existing)?
    } else {
        std::env::current_dir()?
    };
    let mut out = base;
    for comp in tail.into_iter().rev() {
        match Path::new(&comp).components().next() {
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::CurDir) | None => {}
            _ => out.push(comp),
        }
    }
    Ok(out)
}

/// Object-safe handler invoked by [`ToolRegistry::execute`].
pub trait ToolHandler: Send + Sync {
    /// Executes the tool with validated JSON `args`.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] if execution fails.
    fn call(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError>;
}

/// Function-pointer adapter avoiding HRTB inference friction on stateless tool handlers.
struct FnToolHandler(fn(&ToolContext, &Value) -> Result<Value, ToolError>);

impl ToolHandler for FnToolHandler {
    fn call(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        (self.0)(ctx, args)
    }
}

impl<F> ToolHandler for F
where
    F: Fn(&ToolContext, &Value) -> Result<Value, ToolError> + Send + Sync,
{
    fn call(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        self(ctx, args)
    }
}

#[derive(Clone)]
struct RegisteredTool {
    spec: ToolSpec,
    handler: Arc<dyn ToolHandler>,
}

/// One tool invocation in a script passed to [`ToolRegistry::execute_script`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Tool name to invoke.
    pub tool: String,
    /// JSON object of arguments.
    pub arguments: Value,
}

impl ToolCall {
    /// Parses a single tool call object (`{"tool": ..., "arguments": ...}` or `{"name": ..., "arguments": ...}`).
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::InvalidArguments`] if the JSON object lacks a tool name.
    pub fn from_value(value: &Value) -> Result<Self, ToolError> {
        let obj = value.as_object().ok_or_else(|| {
            ToolError::InvalidArguments("tool call entry must be a JSON object".to_owned())
        })?;
        let name = obj
            .get("tool")
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::InvalidArguments("tool call missing 'tool' or 'name' string".to_owned())
            })?;
        let arguments = obj.get("arguments").cloned().unwrap_or_else(|| json!({}));
        Ok(Self {
            tool: name.to_owned(),
            arguments,
        })
    }
}

/// Result of one step in [`ToolRegistry::execute_script`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallResult {
    /// Tool name that ran.
    pub tool: String,
    /// Whether execution succeeded.
    pub ok: bool,
    /// Structured JSON output on success, or `null` on failure.
    pub output: Value,
    /// Error message on failure.
    pub error: Option<String>,
}

impl ToolCallResult {
    /// Serializes this step result as JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "tool": self.tool,
            "ok": self.ok,
            "output": self.output,
            "error": self.error,
        })
    }
}

/// Central registry of all agent-callable tools and their JSON Schemas.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.tools.keys().map(String::as_str).collect();
        f.debug_struct("ToolRegistry")
            .field("tools", &names)
            .finish()
    }
}

impl ToolRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry populated with all built-in core document, search, annotation,
    /// merge/split/compress, render, and redaction tools.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        register_builtin_tools(&mut reg);
        reg
    }

    /// Registers or replaces a tool by name.
    pub fn register(&mut self, spec: ToolSpec, handler: impl ToolHandler + 'static) {
        let name = spec.name.clone();
        self.tools.insert(
            name,
            RegisteredTool {
                spec,
                handler: Arc::new(handler),
            },
        );
    }

    /// Registers or replaces a stateless tool implemented by a function pointer or non-capturing closure.
    pub fn register_fn(
        &mut self,
        spec: ToolSpec,
        handler: fn(&ToolContext, &Value) -> Result<Value, ToolError>,
    ) {
        self.register(spec, FnToolHandler(handler));
    }

    /// Returns all registered tool specs in alphabetical order by name.
    pub fn specs(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.values().map(|t| &t.spec)
    }

    /// Looks up a single tool's specification by `name`.
    #[must_use]
    pub fn spec(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.get(name).map(|t| &t.spec)
    }

    /// Number of registered tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether no tools are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Validates `args` against the tool's `input_schema` and executes it under `ctx`.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::UnknownTool`], [`ToolError::InvalidArguments`], [`ToolError::PathDenied`],
    /// or the handler's own [`ToolError`].
    pub fn execute(&self, ctx: &ToolContext, name: &str, args: &Value) -> Result<Value, ToolError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::UnknownTool(name.to_owned()))?;
        validate_json(args, &tool.spec.input_schema).map_err(ToolError::InvalidArguments)?;
        tool.handler.call(ctx, args)
    }

    /// Runs a sequence of tool calls under `policy`.
    #[must_use]
    pub fn execute_script(
        &self,
        ctx: &ToolContext,
        calls: &[ToolCall],
        policy: ScriptFailurePolicy,
    ) -> Vec<ToolCallResult> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            match self.execute(ctx, &call.tool, &call.arguments) {
                Ok(output) => results.push(ToolCallResult {
                    tool: call.tool.clone(),
                    ok: true,
                    output,
                    error: None,
                }),
                Err(err) => {
                    results.push(ToolCallResult {
                        tool: call.tool.clone(),
                        ok: false,
                        output: Value::Null,
                        error: Some(err.to_string()),
                    });
                    if policy == ScriptFailurePolicy::StopOnError {
                        break;
                    }
                }
            }
        }
        results
    }
}
