//! AI providers, explicit privacy consent gating, redaction filter, and token/cost budget tracking (plan I.1, §12.3).
//!
//! Nothing leaves the device without explicit consent: [`ConsentGate`] enforces global, workspace, and
//! per-document AI switches plus cloud provider approval before any [`AiProvider`] is invoked, and
//! [`RedactionFilter`] strips credentials, emails, phone numbers, and payment identifiers prior to
//! external transmission.

use std::collections::{BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// Errors produced by AI providers, consent checks, redaction, or budget enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ProviderError {
    /// AI is disabled or cloud transmission lacks explicit consent.
    ConsentDenied(String),
    /// Token, request, or cost budget has been exhausted.
    BudgetExceeded {
        /// Cumulative spend in micro-dollars (`1_000_000` = $1.00).
        spent_micros: u64,
        /// Configured spend cap in micro-dollars, if any.
        limit_micros: u64,
        /// Cumulative tokens consumed in the session.
        tokens_used: u64,
        /// Configured token cap, if any.
        token_limit: u64,
    },
    /// Provider endpoint URL is malformed or uses an insecure scheme for a remote host.
    InvalidEndpoint(String),
    /// Underlying HTTP or local IPC transport failed.
    Transport(String),
    /// Remote or local server returned an error response or malformed payload.
    InvalidResponse(String),
    /// Request serialization failed.
    Serialization(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConsentDenied(reason) => write!(f, "ai consent denied: {reason}"),
            Self::BudgetExceeded {
                spent_micros,
                limit_micros,
                tokens_used,
                token_limit,
            } => write!(
                f,
                "ai budget exceeded (spent {spent_micros}/{limit_micros} micros, tokens {tokens_used}/{token_limit})"
            ),
            Self::InvalidEndpoint(msg) => write!(f, "invalid provider endpoint: {msg}"),
            Self::Transport(msg) => write!(f, "provider transport error: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "invalid provider response: {msg}"),
            Self::Serialization(msg) => write!(f, "provider serialization error: {msg}"),
        }
    }
}

impl Error for ProviderError {}

/// Provider implementation family (plan I.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProviderKind {
    /// Local Ollama HTTP server (`/api/chat`, `/api/embed`).
    Ollama,
    /// Local `llama.cpp` HTTP server (`llama-server` OpenAI-compatible API).
    LlamaServer,
    /// Generic OpenAI-compatible HTTP endpoint (local or cloud).
    OpenAiCompatible,
    /// OpenAI Responses / Chat Completions API.
    OpenAi,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Gemini `generateContent` API.
    Gemini,
    /// Deterministic mock provider for automated tests and safety evaluations.
    Test,
}

/// Execution locality surfaced as a visible UI badge (plan §12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Locality {
    /// Runs entirely on the user's machine (`localhost` / `127.0.0.1` or in-process).
    Local,
    /// Sends prompts to a remote cloud endpoint after explicit consent.
    Cloud,
}

impl Locality {
    /// Short badge label for the status bar and chat panel header.
    #[must_use]
    pub fn badge_label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Cloud => "Cloud",
        }
    }
}

/// Global AI consent mode configured in user settings (plan I.1, §12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AiConsentMode {
    /// AI is completely off; no local or cloud AI code runs.
    #[default]
    Disabled,
    /// Only local providers (`Locality::Local`) may run; cloud providers are blocked.
    LocalOnly,
    /// Local providers run; cloud providers require explicit per-provider approval.
    LocalAndCloudWithPrompt,
    /// All configured providers, including approved cloud endpoints, may run.
    CloudApproved,
}

/// Per-document or per-workspace AI override (plan §12.3 off switch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ScopeConsent {
    /// Follow the parent/global [`AiConsentMode`].
    #[default]
    Inherited,
    /// Explicitly turn AI off for this document or workspace.
    ExplicitlyDisabled,
    /// Explicitly allow AI (respecting global mode and cloud approval) for this scope.
    ExplicitlyAllowed,
}

/// Controls whether sensitive tokens are redacted before leaving the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RedactionPolicy {
    /// Automatically redact secrets, emails, phone numbers, SSNs, and card numbers.
    #[default]
    RedactSensitiveData,
    /// Send text verbatim without pattern redaction (for local-only trusted diagnostics).
    AllowUnredacted,
}

/// Role of a message in a chat completion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageRole {
    /// System instructions establishing boundaries and formatting.
    System,
    /// User question or task prompt.
    User,
    /// Prior assistant turn.
    Assistant,
}

impl MessageRole {
    fn as_api_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One message in a provider conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Speaker role.
    pub role: MessageRole,
    /// Message text.
    pub content: String,
}

impl ChatMessage {
    /// Builds a system prompt message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }

    /// Builds a user prompt message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    /// Builds an assistant turn message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

/// Request sent to an [`AiProvider`].
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionRequest {
    /// Target model identifier (provider default if empty).
    pub model: String,
    /// Ordered conversation messages.
    pub messages: Vec<ChatMessage>,
    /// Sampling temperature (`0.0` for deterministic citation tasks).
    pub temperature: f32,
    /// Maximum completion tokens requested.
    pub max_tokens: u32,
    /// Optional JSON Schema requesting structured JSON output (plan I.9).
    pub json_schema: Option<Value>,
}

impl CompletionRequest {
    /// Creates a deterministic completion request with sensible citation defaults.
    #[must_use]
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: 0.0,
            max_tokens: 1024,
            json_schema: None,
        }
    }

    /// Attaches a JSON Schema constraint for structured extraction (plan I.9).
    #[must_use]
    pub fn with_json_schema(mut self, schema: Value) -> Self {
        self.json_schema = Some(schema);
        self
    }

    /// Overrides the maximum completion token limit.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

/// Token and estimated monetary usage reported for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    /// Input prompt tokens.
    pub prompt_tokens: u32,
    /// Generated output tokens.
    pub completion_tokens: u32,
    /// Estimated cost in micro-dollars (`1_000_000` = $1.00 USD; `0` for local models).
    pub estimated_cost_micros: u64,
}

impl TokenUsage {
    /// Sum of prompt and completion tokens.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        u64::from(self.prompt_tokens).saturating_add(u64::from(self.completion_tokens))
    }
}

/// Completed response returned by an [`AiProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResponse {
    /// Generated assistant text.
    pub content: String,
    /// Model identifier that served the request.
    pub model: String,
    /// Token and cost accounting.
    pub usage: TokenUsage,
    /// Optional stop reason (`"stop"`, `"end_turn"`, `"length"`, etc.).
    pub finish_reason: Option<String>,
}

/// Incremental streaming event emitted by [`AiProvider::stream`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamChunk {
    /// Next text fragment generated by the model.
    Delta(String),
    /// Stream completion marker carrying final usage stats.
    Finished(TokenUsage),
}

/// Visible indicator summarizing the active provider, locality, and consent state (plan §12.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBadge {
    /// Provider identifier.
    pub provider_id: String,
    /// Human-readable provider name.
    pub display_name: String,
    /// Configured model name.
    pub model: String,
    /// Local vs Cloud execution badge.
    pub locality: Locality,
    /// Active consent mode.
    pub consent_mode: AiConsentMode,
}

/// Combined global, workspace, and document consent settings (plan I.1, §12.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConsentPolicy {
    /// Global AI consent mode.
    pub global_mode: AiConsentMode,
    /// Active workspace override.
    pub workspace_scope: ScopeConsent,
    /// Active document override.
    pub document_scope: ScopeConsent,
    /// Cloud provider IDs explicitly approved by the user for upload.
    pub approved_cloud_providers: BTreeSet<String>,
    /// Redaction behavior before sending prompts.
    pub redaction: RedactionPolicy,
}

impl ConsentPolicy {
    /// Creates a policy allowing local AI models only.
    #[must_use]
    pub fn local_only() -> Self {
        Self {
            global_mode: AiConsentMode::LocalOnly,
            workspace_scope: ScopeConsent::Inherited,
            document_scope: ScopeConsent::Inherited,
            approved_cloud_providers: BTreeSet::new(),
            redaction: RedactionPolicy::RedactSensitiveData,
        }
    }

    /// Creates a policy explicitly approving one cloud provider ID.
    #[must_use]
    pub fn with_approved_cloud(provider_id: &str) -> Self {
        let mut approved = BTreeSet::new();
        approved.insert(provider_id.to_owned());
        Self {
            global_mode: AiConsentMode::LocalAndCloudWithPrompt,
            workspace_scope: ScopeConsent::Inherited,
            document_scope: ScopeConsent::Inherited,
            approved_cloud_providers: approved,
            redaction: RedactionPolicy::RedactSensitiveData,
        }
    }

    /// Marks `provider_id` as explicitly approved for cloud requests.
    pub fn approve_cloud_provider(&mut self, provider_id: &str) {
        self.approved_cloud_providers.insert(provider_id.to_owned());
    }

    /// Revokes approval for `provider_id`.
    pub fn revoke_cloud_provider(&mut self, provider_id: &str) {
        self.approved_cloud_providers.remove(provider_id);
    }
}

/// Proof token issued by [`ConsentGate::authorize`] confirming policy checks passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedDispatch {
    /// Provider identifier authorized for this request.
    pub provider_id: String,
    /// Locality of the authorized provider.
    pub locality: Locality,
    /// Effective redaction policy to apply before dispatch.
    pub redaction: RedactionPolicy,
}

/// Enforces privacy and explicit consent rules before any AI request executes (plan I.1, §12.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentGate {
    policy: ConsentPolicy,
}

impl ConsentGate {
    /// Creates a gate from `policy`.
    #[must_use]
    pub fn new(policy: ConsentPolicy) -> Self {
        Self { policy }
    }

    /// Current policy inspected by callers.
    #[must_use]
    pub fn policy(&self) -> &ConsentPolicy {
        &self.policy
    }

    /// Mutable access to update scope overrides or approved providers.
    pub fn policy_mut(&mut self) -> &mut ConsentPolicy {
        &mut self.policy
    }

    /// Validates whether `provider_id` at `locality` is permitted to run under the active policy.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::ConsentDenied`] when AI is disabled globally, for the workspace,
    /// for the document, or when cloud usage lacks explicit approval.
    pub fn authorize(
        &self,
        provider_id: &str,
        locality: Locality,
    ) -> Result<AuthorizedDispatch, ProviderError> {
        if self.policy.document_scope == ScopeConsent::ExplicitlyDisabled {
            return Err(ProviderError::ConsentDenied(
                "ai disabled for this document".to_owned(),
            ));
        }
        if self.policy.workspace_scope == ScopeConsent::ExplicitlyDisabled {
            return Err(ProviderError::ConsentDenied(
                "ai disabled for this workspace".to_owned(),
            ));
        }
        match self.policy.global_mode {
            AiConsentMode::Disabled => Err(ProviderError::ConsentDenied(
                "ai is turned off in settings".to_owned(),
            )),
            AiConsentMode::LocalOnly => match locality {
                Locality::Local => Ok(AuthorizedDispatch {
                    provider_id: provider_id.to_owned(),
                    locality,
                    redaction: self.policy.redaction,
                }),
                Locality::Cloud => Err(ProviderError::ConsentDenied(format!(
                    "cloud provider '{provider_id}' blocked in local-only mode"
                ))),
            },
            AiConsentMode::LocalAndCloudWithPrompt => match locality {
                Locality::Local => Ok(AuthorizedDispatch {
                    provider_id: provider_id.to_owned(),
                    locality,
                    redaction: self.policy.redaction,
                }),
                Locality::Cloud => {
                    if self.policy.approved_cloud_providers.contains(provider_id) {
                        Ok(AuthorizedDispatch {
                            provider_id: provider_id.to_owned(),
                            locality,
                            redaction: self.policy.redaction,
                        })
                    } else {
                        Err(ProviderError::ConsentDenied(format!(
                            "cloud provider '{provider_id}' requires explicit user consent"
                        )))
                    }
                }
            },
            AiConsentMode::CloudApproved => Ok(AuthorizedDispatch {
                provider_id: provider_id.to_owned(),
                locality,
                redaction: self.policy.redaction,
            }),
        }
    }
}

/// Kind of sensitive pattern masked by [`RedactionFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum RedactionCategory {
    /// API key or bearer authorization token.
    SecretToken,
    /// Email address.
    EmailAddress,
    /// US Social Security Number pattern (`NNN-NN-NNNN`).
    Ssn,
    /// Payment card number (13 to 19 digits).
    PaymentCard,
    /// Telephone number.
    PhoneNumber,
    /// Caller-configured custom literal term.
    CustomTerm,
}

/// Result of running [`RedactionFilter::sanitize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionReport {
    /// Text safe to send after redaction.
    pub sanitized: String,
    /// Total number of replacements made.
    pub redactions_applied: usize,
    /// Distinct categories triggered, sorted deterministically.
    pub categories: Vec<RedactionCategory>,
}

/// Strips secrets, PII, and custom sensitive terms before sending document text to external providers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionFilter {
    blocked_terms: Vec<String>,
}

impl RedactionFilter {
    /// Creates a filter with built-in PII and credential rules.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a custom sensitive literal (for example, a client name or internal project code) to mask.
    pub fn add_blocked_term(&mut self, term: &str) {
        let trimmed = term.trim();
        if !trimmed.is_empty() && !self.blocked_terms.iter().any(|t| t == trimmed) {
            self.blocked_terms.push(trimmed.to_owned());
        }
    }

    /// Masks sensitive substrings in `input` and reports every category triggered.
    #[must_use]
    pub fn sanitize(&self, input: &str) -> RedactionReport {
        let mut text = input.to_owned();
        let mut count = 0usize;
        let mut cats = BTreeSet::new();

        for term in &self.blocked_terms {
            if !term.is_empty() && text.contains(term) {
                let matches = text.matches(term).count();
                count = count.saturating_add(matches);
                text = text.replace(term, "[REDACTED_TERM]");
                cats.insert(RedactionCategory::CustomTerm);
            }
        }

        let (after_bearer, bearer_n) = redact_bearer_tokens(&text);
        if bearer_n > 0 {
            text = after_bearer;
            count = count.saturating_add(bearer_n);
            cats.insert(RedactionCategory::SecretToken);
        }

        let (after_keys, key_n) = redact_api_keys(&text);
        if key_n > 0 {
            text = after_keys;
            count = count.saturating_add(key_n);
            cats.insert(RedactionCategory::SecretToken);
        }

        let (after_email, email_n) = redact_emails(&text);
        if email_n > 0 {
            text = after_email;
            count = count.saturating_add(email_n);
            cats.insert(RedactionCategory::EmailAddress);
        }

        let (after_ssn, ssn_n) = redact_ssns(&text);
        if ssn_n > 0 {
            text = after_ssn;
            count = count.saturating_add(ssn_n);
            cats.insert(RedactionCategory::Ssn);
        }

        let (after_cards, card_n) = redact_cards(&text);
        if card_n > 0 {
            text = after_cards;
            count = count.saturating_add(card_n);
            cats.insert(RedactionCategory::PaymentCard);
        }

        let (after_phones, phone_n) = redact_phones(&text);
        if phone_n > 0 {
            text = after_phones;
            count = count.saturating_add(phone_n);
            cats.insert(RedactionCategory::PhoneNumber);
        }

        RedactionReport {
            sanitized: text,
            redactions_applied: count,
            categories: cats.into_iter().collect(),
        }
    }
}

fn redact_bearer_tokens(input: &str) -> (String, usize) {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut count = 0usize;
    while let Some(pos) = rest.find("Bearer ") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + "Bearer ".len()..];
        let token_len = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            .count();
        if token_len >= 8 {
            out.push_str("Bearer [REDACTED_SECRET]");
            rest = &after[token_len..];
            count += 1;
        } else {
            out.push_str("Bearer ");
            rest = after;
        }
    }
    out.push_str(rest);
    (out, count)
}

fn redact_api_keys(input: &str) -> (String, usize) {
    const PREFIXES: &[&str] = &["sk-ant-", "sk-proj-", "sk-", "AIza", "ghp_", "xoxb-"];
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut count = 0usize;
    let bytes = input.as_bytes();
    while i < input.len() {
        let slice = &input[i..];
        if let Some(prefix) = PREFIXES.iter().find(|p| slice.starts_with(**p)) {
            let boundary_ok =
                i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let tok_len = slice
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                .count();
            if boundary_ok && tok_len >= prefix.len() + 10 {
                out.push_str("[REDACTED_SECRET]");
                i += tok_len;
                count += 1;
                continue;
            }
        }
        let ch = slice
            .chars()
            .next()
            .expect("i < input.len() guarantees at least one char");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

fn redact_emails(input: &str) -> (String, usize) {
    let mut out = String::with_capacity(input.len());
    let mut count = 0usize;
    let mut start = 0usize;
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let mut left = i;
            while left > start && is_email_local(bytes[left - 1]) {
                left -= 1;
            }
            let mut right = i + 1;
            while right < bytes.len() && is_email_domain(bytes[right]) {
                right += 1;
            }
            let domain = &input[i + 1..right];
            let valid_domain = domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && domain.len() >= 3;
            if left < i && valid_domain {
                out.push_str(&input[start..left]);
                out.push_str("[REDACTED_EMAIL]");
                start = right;
                i = right;
                count += 1;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[start..]);
    (out, count)
}

fn is_email_local(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+')
}

fn is_email_domain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

fn redact_ssns(input: &str) -> (String, usize) {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() {
        if i + 11 <= bytes.len()
            && bytes[i..i + 3].iter().all(u8::is_ascii_digit)
            && bytes[i + 3] == b'-'
            && bytes[i + 4..i + 6].iter().all(u8::is_ascii_digit)
            && bytes[i + 6] == b'-'
            && bytes[i + 7..i + 11].iter().all(u8::is_ascii_digit)
            && (i == 0 || !bytes[i - 1].is_ascii_digit())
            && (i + 11 == bytes.len() || !bytes[i + 11].is_ascii_digit())
        {
            out.push_str("[REDACTED_SSN]");
            i += 11;
            count += 1;
            continue;
        }
        let ch = input[i..]
            .chars()
            .next()
            .expect("i < bytes.len() guarantees at least one char");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

fn redact_cards(input: &str) -> (String, usize) {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_digit()) {
            let mut end = i;
            let mut digits = 0usize;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || bytes[end] == b'-' || bytes[end] == b' ')
            {
                if bytes[end].is_ascii_digit() {
                    digits += 1;
                }
                end += 1;
            }
            let trimmed_end = input[i..end].trim_end_matches([' ', '-']).len() + i;
            if (13..=19).contains(&digits) && luhn_valid(&input[i..trimmed_end]) {
                out.push_str("[REDACTED_CARD]");
                i = trimmed_end;
                count += 1;
                continue;
            }
        }
        let ch = input[i..]
            .chars()
            .next()
            .expect("i < bytes.len() guarantees at least one char");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

fn luhn_valid(candidate: &str) -> bool {
    let digits: Vec<u32> = candidate
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| u32::from(b - b'0'))
        .collect();
    if digits.len() < 13 {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(idx, &d)| {
            if idx % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn redact_phones(input: &str) -> (String, usize) {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() {
        let starts_phone = bytes[i] == b'+'
            || bytes[i] == b'('
            || (bytes[i].is_ascii_digit()
                && i + 10 <= bytes.len()
                && bytes[i..].iter().take(4).any(|b| *b == b'-'));
        if starts_phone && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let mut end = i;
            let mut digits = 0usize;
            let mut sep_count = 0usize;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit()
                    || matches!(bytes[end], b'+' | b'-' | b'(' | b')' | b' '))
            {
                if bytes[end].is_ascii_digit() {
                    digits += 1;
                } else if matches!(bytes[end], b'-' | b'(' | b')') {
                    sep_count += 1;
                }
                end += 1;
            }
            let trimmed_end = input[i..end].trim_end_matches([' ', '-', '(', ')']).len() + i;
            if (10..=15).contains(&digits) && sep_count >= 1 {
                out.push_str("[REDACTED_PHONE]");
                i = trimmed_end;
                count += 1;
                continue;
            }
        }
        let ch = input[i..]
            .chars()
            .next()
            .expect("i < bytes.len() guarantees at least one char");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

/// Configured session caps for AI token, request, and monetary spend (plan I.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageBudget {
    /// Maximum total prompt + completion tokens allowed (`None` = unlimited).
    pub max_total_tokens: Option<u64>,
    /// Maximum spend in micro-dollars (`None` = unlimited).
    pub max_cost_micros: Option<u64>,
    /// Maximum requests per session (`None` = unlimited).
    pub max_requests: Option<u32>,
}

/// Snapshot of cumulative usage recorded by a [`BudgetTracker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSnapshot {
    /// Cumulative prompt tokens consumed.
    pub prompt_tokens: u64,
    /// Cumulative completion tokens generated.
    pub completion_tokens: u64,
    /// Cumulative spend in micro-dollars.
    pub spent_micros: u64,
    /// Total requests executed.
    pub request_count: u32,
    /// Total redactions applied prior to dispatch.
    pub redactions_applied: u64,
}

impl UsageSnapshot {
    /// Combined prompt and completion tokens consumed.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}

/// Tracks cumulative token usage and enforces [`UsageBudget`] limits.
#[derive(Debug, Clone, Default)]
pub struct BudgetTracker {
    budget: UsageBudget,
    snapshot: UsageSnapshot,
}

impl BudgetTracker {
    /// Creates a new tracker governed by `budget`.
    #[must_use]
    pub fn new(budget: UsageBudget) -> Self {
        Self {
            budget,
            snapshot: UsageSnapshot::default(),
        }
    }

    /// Configured budget limits.
    #[must_use]
    pub fn budget(&self) -> UsageBudget {
        self.budget
    }

    /// Cumulative usage snapshot so far.
    #[must_use]
    pub fn snapshot(&self) -> UsageSnapshot {
        self.snapshot
    }

    /// Verifies that another request with `estimated_tokens` and `estimated_cost_micros` stays within budget.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::BudgetExceeded`] if any configured limit would be exceeded.
    pub fn check_can_dispatch(
        &self,
        estimated_tokens: u64,
        estimated_cost_micros: u64,
    ) -> Result<(), ProviderError> {
        let current_tokens = self.snapshot.total_tokens();
        let next_tokens = current_tokens.saturating_add(estimated_tokens);
        let next_cost = self
            .snapshot
            .spent_micros
            .saturating_add(estimated_cost_micros);

        if let Some(limit) = self.budget.max_requests
            && self.snapshot.request_count >= limit
        {
            return Err(ProviderError::BudgetExceeded {
                spent_micros: self.snapshot.spent_micros,
                limit_micros: self.budget.max_cost_micros.unwrap_or(0),
                tokens_used: current_tokens,
                token_limit: self.budget.max_total_tokens.unwrap_or(0),
            });
        }
        if let Some(limit) = self.budget.max_total_tokens
            && next_tokens > limit
        {
            return Err(ProviderError::BudgetExceeded {
                spent_micros: self.snapshot.spent_micros,
                limit_micros: self.budget.max_cost_micros.unwrap_or(0),
                tokens_used: current_tokens,
                token_limit: limit,
            });
        }
        if let Some(limit) = self.budget.max_cost_micros
            && next_cost > limit
        {
            return Err(ProviderError::BudgetExceeded {
                spent_micros: self.snapshot.spent_micros,
                limit_micros: limit,
                tokens_used: current_tokens,
                token_limit: self.budget.max_total_tokens.unwrap_or(0),
            });
        }
        Ok(())
    }

    /// Records completed request usage and applied redactions.
    pub fn record(&mut self, usage: &TokenUsage, redactions_applied: usize) {
        self.snapshot.prompt_tokens = self
            .snapshot
            .prompt_tokens
            .saturating_add(u64::from(usage.prompt_tokens));
        self.snapshot.completion_tokens = self
            .snapshot
            .completion_tokens
            .saturating_add(u64::from(usage.completion_tokens));
        self.snapshot.spent_micros = self
            .snapshot
            .spent_micros
            .saturating_add(usage.estimated_cost_micros);
        self.snapshot.request_count = self.snapshot.request_count.saturating_add(1);
        self.snapshot.redactions_applied = self
            .snapshot
            .redactions_applied
            .saturating_add(u64::try_from(redactions_applied).unwrap_or(u64::MAX));
    }
}

/// Core trait implemented by local, cloud, and test AI providers (plan I.1).
pub trait AiProvider: Send + Sync {
    /// Stable identifier used in consent rules and settings (e.g. `"ollama"`, `"openai"`).
    fn id(&self) -> &str;

    /// Human-readable display name shown in the UI.
    fn display_name(&self) -> &str;

    /// Provider backend family.
    fn kind(&self) -> ProviderKind;

    /// Whether inference runs locally or in the cloud.
    fn locality(&self) -> Locality;

    /// Default model name when a request leaves `model` empty.
    fn default_model(&self) -> &str;

    /// Executes a synchronous chat completion request.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the transport fails or the provider rejects the request.
    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError>;

    /// Streams completion chunks to `on_chunk` and returns the combined response.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the transport fails or the provider rejects the request.
    fn stream(
        &self,
        request: &CompletionRequest,
        on_chunk: &mut dyn FnMut(StreamChunk),
    ) -> Result<CompletionResponse, ProviderError> {
        let response = self.complete(request)?;
        if !response.content.is_empty() {
            on_chunk(StreamChunk::Delta(response.content.clone()));
        }
        on_chunk(StreamChunk::Finished(response.usage));
        Ok(response)
    }

    /// Generates dense embeddings for `texts`; defaults to a deterministic local hashed TF-IDF vector.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if a remote embedding call fails.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ProviderError> {
        Ok(texts
            .iter()
            .map(|text| crate::index::hashed_text_embedding(text))
            .collect())
    }
}

/// Wraps an [`AiProvider`] with [`ConsentGate`], [`RedactionFilter`], and [`BudgetTracker`].
pub struct GuardedProvider<'a> {
    provider: &'a dyn AiProvider,
    gate: ConsentGate,
    redaction: RedactionFilter,
    budget: BudgetTracker,
}

impl fmt::Debug for GuardedProvider<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardedProvider")
            .field("provider_id", &self.provider.id())
            .field("locality", &self.provider.locality())
            .field("consent_mode", &self.gate.policy().global_mode)
            .finish_non_exhaustive()
    }
}

impl<'a> GuardedProvider<'a> {
    /// Creates a guarded dispatch channel around `provider`.
    #[must_use]
    pub fn new(provider: &'a dyn AiProvider, policy: ConsentPolicy, budget: UsageBudget) -> Self {
        Self {
            provider,
            gate: ConsentGate::new(policy),
            redaction: RedactionFilter::new(),
            budget: BudgetTracker::new(budget),
        }
    }

    /// Attaches a custom redaction filter.
    #[must_use]
    pub fn with_redaction_filter(mut self, filter: RedactionFilter) -> Self {
        self.redaction = filter;
        self
    }

    /// Current visible badge describing locality, model, and consent state.
    #[must_use]
    pub fn badge(&self) -> ProviderBadge {
        ProviderBadge {
            provider_id: self.provider.id().to_owned(),
            display_name: self.provider.display_name().to_owned(),
            model: self.provider.default_model().to_owned(),
            locality: self.provider.locality(),
            consent_mode: self.gate.policy().global_mode,
        }
    }

    /// Current cumulative token and cost snapshot.
    #[must_use]
    pub fn usage(&self) -> UsageSnapshot {
        self.budget.snapshot()
    }

    /// Active consent policy.
    #[must_use]
    pub fn policy(&self) -> &ConsentPolicy {
        self.gate.policy()
    }

    /// Mutable access to the underlying consent policy.
    pub fn policy_mut(&mut self) -> &mut ConsentPolicy {
        self.gate.policy_mut()
    }

    /// Checks consent, enforces budget, applies redaction, and dispatches `request`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::ConsentDenied`] if unauthorized (guaranteed zero provider calls),
    /// [`ProviderError::BudgetExceeded`] if limits are hit, or transport/response errors.
    pub fn complete(
        &mut self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let auth = self
            .gate
            .authorize(self.provider.id(), self.provider.locality())?;
        let est_tokens = estimate_request_tokens(request);
        self.budget.check_can_dispatch(est_tokens, 0)?;

        let (sanitized_req, redacted_count) = self.prepare_request(request, auth.redaction);
        let response = self.provider.complete(&sanitized_req)?;
        self.budget.record(&response.usage, redacted_count);
        Ok(response)
    }

    /// Checks consent, enforces budget, applies redaction, and streams `request`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if consent fails, budget is exceeded, or streaming fails.
    pub fn stream(
        &mut self,
        request: &CompletionRequest,
        on_chunk: &mut dyn FnMut(StreamChunk),
    ) -> Result<CompletionResponse, ProviderError> {
        let auth = self
            .gate
            .authorize(self.provider.id(), self.provider.locality())?;
        let est_tokens = estimate_request_tokens(request);
        self.budget.check_can_dispatch(est_tokens, 0)?;

        let (sanitized_req, redacted_count) = self.prepare_request(request, auth.redaction);
        let response = self.provider.stream(&sanitized_req, on_chunk)?;
        self.budget.record(&response.usage, redacted_count);
        Ok(response)
    }

    fn prepare_request(
        &self,
        request: &CompletionRequest,
        policy: RedactionPolicy,
    ) -> (CompletionRequest, usize) {
        if policy == RedactionPolicy::AllowUnredacted {
            return (request.clone(), 0);
        }
        let mut total_redactions = 0usize;
        let messages = request
            .messages
            .iter()
            .map(|msg| {
                let report = self.redaction.sanitize(&msg.content);
                total_redactions = total_redactions.saturating_add(report.redactions_applied);
                ChatMessage {
                    role: msg.role,
                    content: report.sanitized,
                }
            })
            .collect();
        let mut out = request.clone();
        out.messages = messages;
        (out, total_redactions)
    }
}

fn estimate_request_tokens(request: &CompletionRequest) -> u64 {
    let chars: usize = request.messages.iter().map(|m| m.content.len()).sum();
    u64::try_from(chars / 4 + 1).unwrap_or(u64::MAX)
}

fn validate_endpoint_url(url: &str, locality: Locality) -> Result<String, ProviderError> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ProviderError::InvalidEndpoint(
            "endpoint url cannot be empty".to_owned(),
        ));
    }
    let is_https = trimmed.starts_with("https://");
    let is_local_http = trimmed.starts_with("http://127.0.0.1")
        || trimmed.starts_with("http://localhost")
        || trimmed.starts_with("http://[::1]");
    match locality {
        Locality::Local => {
            if is_https || is_local_http {
                Ok(trimmed.to_owned())
            } else {
                Err(ProviderError::InvalidEndpoint(format!(
                    "local provider must target localhost or https: {trimmed}"
                )))
            }
        }
        Locality::Cloud => {
            if is_https {
                Ok(trimmed.to_owned())
            } else {
                Err(ProviderError::InvalidEndpoint(format!(
                    "cloud provider requires https: {trimmed}"
                )))
            }
        }
    }
}

fn infer_locality(base_url: &str) -> Locality {
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.starts_with("http://127.0.0.1")
        || lower.starts_with("http://localhost")
        || lower.starts_with("http://[::1]")
        || lower.starts_with("https://127.0.0.1")
        || lower.starts_with("https://localhost")
    {
        Locality::Local
    } else {
        Locality::Cloud
    }
}

/// OpenAI-compatible HTTP provider covering OpenAI, `llama-server`, and custom endpoints (plan I.1).
#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    id: String,
    display_name: String,
    kind: ProviderKind,
    locality: Locality,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("locality", &self.locality)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    /// Creates a local `llama-server` provider on `base_url` (e.g. `"http://127.0.0.1:8080/v1"`).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidEndpoint`] if `base_url` is not loopback or HTTPS.
    pub fn llama_server(base_url: &str, model: &str) -> Result<Self, ProviderError> {
        let valid = validate_endpoint_url(base_url, Locality::Local)?;
        Ok(Self {
            id: "llama-server".to_owned(),
            display_name: "llama.cpp Server".to_owned(),
            kind: ProviderKind::LlamaServer,
            locality: Locality::Local,
            base_url: valid,
            api_key: None,
            model: model.to_owned(),
        })
    }

    /// Creates an OpenAI cloud provider using `api_key` and `model`.
    #[must_use]
    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: "openai".to_owned(),
            display_name: "OpenAI".to_owned(),
            kind: ProviderKind::OpenAi,
            locality: Locality::Cloud,
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key: Some(api_key.into()),
            model: model.into(),
        }
    }

    /// Creates a custom OpenAI-compatible provider, inferring [`Locality`] from `base_url`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidEndpoint`] if a remote URL is not HTTPS.
    pub fn custom(
        id: &str,
        display_name: &str,
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
    ) -> Result<Self, ProviderError> {
        let locality = infer_locality(base_url);
        let valid = validate_endpoint_url(base_url, locality)?;
        Ok(Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            kind: ProviderKind::OpenAiCompatible,
            locality,
            base_url: valid,
            api_key: api_key.map(str::to_owned),
            model: model.to_owned(),
        })
    }

    /// Parses an OpenAI `/chat/completions` JSON payload into a [`CompletionResponse`].
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidResponse`] if required fields are absent or malformed.
    pub fn parse_chat_response(raw_json: &str) -> Result<CompletionResponse, ProviderError> {
        let val: Value = serde_json::from_str(raw_json)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid json: {e}")))?;
        let choice = val
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .ok_or_else(|| ProviderError::InvalidResponse("missing choices[0]".to_owned()))?;
        let content = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("missing choices[0].message.content".to_owned())
            })?
            .to_owned();
        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let model = val
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let usage = parse_openai_usage(val.get("usage"));
        Ok(CompletionResponse {
            content,
            model,
            usage,
            finish_reason,
        })
    }

    /// Parses OpenAI Server-Sent Events lines into incremental [`StreamChunk`]s.
    #[must_use]
    pub fn parse_sse_lines(raw: &str) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            let Some(data) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                chunks.push(StreamChunk::Finished(TokenUsage::default()));
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(data)
                && let Some(delta) = val
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(Value::as_str)
                && !delta.is_empty()
            {
                chunks.push(StreamChunk::Delta(delta.to_owned()));
            }
        }
        chunks
    }
}

fn parse_openai_usage(usage: Option<&Value>) -> TokenUsage {
    let Some(u) = usage else {
        return TokenUsage::default();
    };
    let prompt_tokens = u
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let completion_tokens = u
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    TokenUsage {
        prompt_tokens,
        completion_tokens,
        estimated_cost_micros: 0,
    }
}

impl AiProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn locality(&self) -> Locality {
        self.locality
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let model = if request.model.is_empty() {
            &self.model
        } else {
            &request.model
        };
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_api_str(),
                    "content": m.content,
                })
            })
            .collect();
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        });
        if let Some(schema) = &request.json_schema
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert(
                "response_format".to_owned(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "structured_response",
                        "strict": true,
                        "schema": schema,
                    }
                }),
            );
        }

        let url = format!("{}/chat/completions", self.base_url);
        let mut req = ureq::post(&url).header("Content-Type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let mut resp = req
            .send(payload.to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::parse_chat_response(&body)
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/embeddings", self.base_url);
        let payload = json!({
            "model": self.model,
            "input": texts,
        });
        let mut req = ureq::post(&url).header("Content-Type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let mut resp = req
            .send(payload.to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let val: Value = serde_json::from_str(&body)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid embed json: {e}")))?;
        let data = val
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidResponse("missing embeddings data".to_owned()))?;
        let mut out = Vec::with_capacity(data.len());
        for item in data {
            let arr = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("missing embedding array".to_owned())
                })?;
            let vec: Vec<f32> = arr
                .iter()
                .filter_map(|v| v.as_f64().map(|x| x as f32))
                .collect();
            out.push(vec);
        }
        Ok(out)
    }
}

/// Local Ollama provider targeting `/api/chat` and `/api/embed` (plan I.1).
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    base_url: String,
    model: String,
}

impl OllamaProvider {
    /// Creates an Ollama provider targeting `base_url` (typically `"http://127.0.0.1:11434"`).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidEndpoint`] if `base_url` is not loopback or HTTPS.
    pub fn new(base_url: &str, model: &str) -> Result<Self, ProviderError> {
        let valid = validate_endpoint_url(base_url, Locality::Local)?;
        Ok(Self {
            base_url: valid,
            model: model.to_owned(),
        })
    }

    /// Parses a non-streaming Ollama `/api/chat` JSON payload.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidResponse`] if required fields are absent or malformed.
    pub fn parse_chat_response(raw_json: &str) -> Result<CompletionResponse, ProviderError> {
        let val: Value = serde_json::from_str(raw_json)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid ollama json: {e}")))?;
        let content = val
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("missing message.content in ollama reply".to_owned())
            })?
            .to_owned();
        let model = val
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("ollama")
            .to_owned();
        let prompt_tokens = val
            .get("prompt_eval_count")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let completion_tokens = val
            .get("eval_count")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let finish_reason = val
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(CompletionResponse {
            content,
            model,
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                estimated_cost_micros: 0,
            },
            finish_reason,
        })
    }
}

impl AiProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }

    fn display_name(&self) -> &str {
        "Ollama (Local)"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    fn locality(&self) -> Locality {
        Locality::Local
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let model = if request.model.is_empty() {
            &self.model
        } else {
            &request.model
        };
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_api_str(),
                    "content": m.content,
                })
            })
            .collect();
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": {
                "temperature": request.temperature,
                "num_predict": request.max_tokens,
            }
        });
        if let Some(schema) = &request.json_schema
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("format".to_owned(), schema.clone());
        }
        let url = format!("{}/api/chat", self.base_url);
        let mut resp = ureq::post(&url)
            .header("Content-Type", "application/json")
            .send(payload.to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::parse_chat_response(&body)
    }
}

/// Anthropic Messages API cloud provider (plan I.1).
#[derive(Clone)]
pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .finish()
    }
}

impl AnthropicProvider {
    /// Creates an Anthropic cloud provider using `api_key` and `model`.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Parses an Anthropic `/v1/messages` JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidResponse`] if required fields are absent or malformed.
    pub fn parse_messages_response(raw_json: &str) -> Result<CompletionResponse, ProviderError> {
        let val: Value = serde_json::from_str(raw_json)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid anthropic json: {e}")))?;
        let content_blocks = val
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidResponse("missing content array".to_owned()))?;
        let mut combined = String::new();
        for block in content_blocks {
            if block.get("type").and_then(Value::as_str) == Some("text")
                && let Some(text) = block.get("text").and_then(Value::as_str)
            {
                combined.push_str(text);
            }
        }
        let model = val
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("claude")
            .to_owned();
        let prompt_tokens = val
            .get("usage")
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let completion_tokens = val
            .get("usage")
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let finish_reason = val
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(CompletionResponse {
            content: combined,
            model,
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                estimated_cost_micros: 0,
            },
            finish_reason,
        })
    }
}

impl AiProvider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn display_name(&self) -> &str {
        "Anthropic"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn locality(&self) -> Locality {
        Locality::Cloud
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let model = if request.model.is_empty() {
            &self.model
        } else {
            &request.model
        };
        let system_prompt: String = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages: Vec<Value> = request
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| {
                json!({
                    "role": m.role.as_api_str(),
                    "content": m.content,
                })
            })
            .collect();
        let mut payload = json!({
            "model": model,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "messages": messages,
        });
        if !system_prompt.is_empty()
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("system".to_owned(), Value::String(system_prompt));
        }

        let url = format!("{}/v1/messages", self.base_url);
        let mut resp = ureq::post(&url)
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send(payload.to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::parse_messages_response(&body)
    }
}

/// Google Gemini `generateContent` cloud provider (plan I.1).
#[derive(Clone)]
pub struct GeminiProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .finish()
    }
}

impl GeminiProvider {
    /// Creates a Gemini cloud provider using `api_key` and `model`.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://generativelanguage.googleapis.com".to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Parses a Gemini `:generateContent` JSON response.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidResponse`] if required fields are absent or malformed.
    pub fn parse_generate_response(raw_json: &str) -> Result<CompletionResponse, ProviderError> {
        let val: Value = serde_json::from_str(raw_json)
            .map_err(|e| ProviderError::InvalidResponse(format!("invalid gemini json: {e}")))?;
        let candidate = val
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .ok_or_else(|| ProviderError::InvalidResponse("missing candidates[0]".to_owned()))?;
        let parts = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("missing candidates[0].content.parts".to_owned())
            })?;
        let mut combined = String::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
        let finish_reason = candidate
            .get("finishReason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let usage_meta = val.get("usageMetadata");
        let prompt_tokens = usage_meta
            .and_then(|u| u.get("promptTokenCount"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let completion_tokens = usage_meta
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        Ok(CompletionResponse {
            content: combined,
            model: "gemini".to_owned(),
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                estimated_cost_micros: 0,
            },
            finish_reason,
        })
    }
}

impl AiProvider for GeminiProvider {
    fn id(&self) -> &str {
        "gemini"
    }

    fn display_name(&self) -> &str {
        "Google Gemini"
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Gemini
    }

    fn locality(&self) -> Locality {
        Locality::Cloud
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let model = if request.model.is_empty() {
            &self.model
        } else {
            &request.model
        };
        let system_prompt: String = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let contents: Vec<Value> = request
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| {
                let role = match m.role {
                    MessageRole::Assistant => "model",
                    MessageRole::System | MessageRole::User => "user",
                };
                json!({
                    "role": role,
                    "parts": [{ "text": m.content }],
                })
            })
            .collect();
        let mut generation_config = json!({
            "temperature": request.temperature,
            "maxOutputTokens": request.max_tokens,
        });
        if let Some(schema) = &request.json_schema
            && let Some(obj) = generation_config.as_object_mut()
        {
            obj.insert(
                "responseMimeType".to_owned(),
                Value::String("application/json".to_owned()),
            );
            obj.insert("responseSchema".to_owned(), schema.clone());
        }
        let mut payload = json!({
            "contents": contents,
            "generationConfig": generation_config,
        });
        if !system_prompt.is_empty()
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert(
                "systemInstruction".to_owned(),
                json!({ "parts": [{ "text": system_prompt }] }),
            );
        }

        let url = format!("{}/v1beta/models/{model}:generateContent", self.base_url);
        let mut resp = ureq::post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.api_key)
            .send(payload.to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::parse_generate_response(&body)
    }
}

type TestResponder =
    Arc<dyn Fn(&CompletionRequest) -> Result<CompletionResponse, ProviderError> + Send + Sync>;

#[derive(Default)]
struct TestProviderState {
    recorded: Vec<CompletionRequest>,
    queued: VecDeque<Result<CompletionResponse, ProviderError>>,
}

/// Deterministic mockable provider for tests and safety evaluations (plan I.1, I.12).
#[derive(Clone)]
pub struct TestProvider {
    id: String,
    display_name: String,
    locality: Locality,
    model: String,
    state: Arc<Mutex<TestProviderState>>,
    responder: Option<TestResponder>,
}

impl fmt::Debug for TestProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestProvider")
            .field("id", &self.id)
            .field("locality", &self.locality)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl TestProvider {
    /// Creates a mock provider at `locality` with a deterministic default reply.
    #[must_use]
    pub fn new(id: &str, locality: Locality) -> Self {
        Self {
            id: id.to_owned(),
            display_name: format!("Test ({id})"),
            locality,
            model: "test-model".to_owned(),
            state: Arc::new(Mutex::new(TestProviderState::default())),
            responder: None,
        }
    }

    /// Attaches a custom response closure invoked on each request.
    #[must_use]
    pub fn with_responder<F>(mut self, responder: F) -> Self
    where
        F: Fn(&CompletionRequest) -> Result<CompletionResponse, ProviderError>
            + Send
            + Sync
            + 'static,
    {
        self.responder = Some(Arc::new(responder));
        self
    }

    /// Enqueues a scripted response to return on the next call.
    pub fn push_response(&self, content: impl Into<String>, usage: TokenUsage) {
        let resp = CompletionResponse {
            content: content.into(),
            model: self.model.clone(),
            usage,
            finish_reason: Some("stop".to_owned()),
        };
        let mut guard = self.state.lock().expect("test provider mutex poisoned");
        guard.queued.push_back(Ok(resp));
    }

    /// Enqueues an explicit error to return on the next call.
    pub fn push_error(&self, err: ProviderError) {
        let mut guard = self.state.lock().expect("test provider mutex poisoned");
        guard.queued.push_back(Err(err));
    }

    /// Total number of requests dispatched to this provider.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.state
            .lock()
            .expect("test provider mutex poisoned")
            .recorded
            .len()
    }

    /// Snapshot of every [`CompletionRequest`] received by this provider.
    #[must_use]
    pub fn recorded_requests(&self) -> Vec<CompletionRequest> {
        self.state
            .lock()
            .expect("test provider mutex poisoned")
            .recorded
            .clone()
    }
}

impl AiProvider for TestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Test
    }

    fn locality(&self) -> Locality {
        self.locality
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let queued = {
            let mut guard = self.state.lock().expect("test provider mutex poisoned");
            guard.recorded.push(request.clone());
            guard.queued.pop_front()
        };
        if let Some(res) = queued {
            return res;
        }
        if let Some(responder) = &self.responder {
            return responder(request);
        }
        Ok(CompletionResponse {
            content: r#"{"claims":[],"insufficient_evidence":true}"#.to_owned(),
            model: self.model.clone(),
            usage: TokenUsage {
                prompt_tokens: 16,
                completion_tokens: 8,
                estimated_cost_micros: 0,
            },
            finish_reason: Some("stop".to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_gate_blocks_all_calls_when_disabled_or_unapproved_cloud() {
        let mock_cloud = TestProvider::new("openai", Locality::Cloud);
        mock_cloud.push_response("ok", TokenUsage::default());

        let mut disabled = GuardedProvider::new(
            &mock_cloud,
            ConsentPolicy::default(),
            UsageBudget::default(),
        );
        let req = CompletionRequest::new("gpt", vec![ChatMessage::user("hello")]);
        let err = disabled.complete(&req).expect_err("disabled must block");
        assert!(matches!(err, ProviderError::ConsentDenied(_)));
        assert_eq!(mock_cloud.call_count(), 0, "zero network calls allowed");

        let mut local_only = GuardedProvider::new(
            &mock_cloud,
            ConsentPolicy::local_only(),
            UsageBudget::default(),
        );
        let err = local_only
            .complete(&req)
            .expect_err("local-only must block cloud");
        assert!(matches!(err, ProviderError::ConsentDenied(_)));
        assert_eq!(mock_cloud.call_count(), 0, "zero cloud calls allowed");

        local_only.policy_mut().approve_cloud_provider("openai");
        local_only.policy_mut().global_mode = AiConsentMode::LocalAndCloudWithPrompt;
        let resp = local_only.complete(&req).expect("approved cloud succeeds");
        assert_eq!(resp.content, "ok");
        assert_eq!(mock_cloud.call_count(), 1);
    }

    #[test]
    fn document_and_workspace_switches_override_global_consent() {
        let local = TestProvider::new("ollama", Locality::Local);
        let mut policy = ConsentPolicy::local_only();
        policy.document_scope = ScopeConsent::ExplicitlyDisabled;
        let mut guarded = GuardedProvider::new(&local, policy, UsageBudget::default());
        let req = CompletionRequest::new("llama", vec![ChatMessage::user("summarize")]);
        let err = guarded.complete(&req).expect_err("doc switch blocks");
        assert!(matches!(err, ProviderError::ConsentDenied(_)));
        assert_eq!(local.call_count(), 0);
    }

    #[test]
    fn redaction_filter_masks_secrets_and_pii_before_dispatch() {
        let cloud = TestProvider::new("openai", Locality::Cloud);
        cloud.push_response("done", TokenUsage::default());
        let mut filter = RedactionFilter::new();
        filter.add_blocked_term("Project Nightingale");

        let mut guarded = GuardedProvider::new(
            &cloud,
            ConsentPolicy::with_approved_cloud("openai"),
            UsageBudget::default(),
        )
        .with_redaction_filter(filter);

        let req = CompletionRequest::new(
            "gpt-4o",
            vec![ChatMessage::user(
                "Contact alice@example.com or +1-555-234-5678 about Project Nightingale with sk-proj-1234567890abcdef and SSN 123-45-6789",
            )],
        );
        guarded.complete(&req).expect("dispatches redacted prompt");
        let sent = &cloud.recorded_requests()[0].messages[0].content;
        assert!(!sent.contains("alice@example.com"));
        assert!(!sent.contains("sk-proj-1234567890abcdef"));
        assert!(!sent.contains("123-45-6789"));
        assert!(!sent.contains("Project Nightingale"));
        assert!(sent.contains("[REDACTED_EMAIL]"));
        assert!(sent.contains("[REDACTED_SECRET]"));
        assert!(sent.contains("[REDACTED_SSN]"));
        assert!(sent.contains("[REDACTED_TERM]"));
        assert!(guarded.usage().redactions_applied >= 4);
    }

    #[test]
    fn budget_tracker_enforces_token_and_request_caps() {
        let local = TestProvider::new("llama-server", Locality::Local);
        local.push_response(
            "first",
            TokenUsage {
                prompt_tokens: 40,
                completion_tokens: 20,
                estimated_cost_micros: 0,
            },
        );
        let budget = UsageBudget {
            max_total_tokens: Some(65),
            max_cost_micros: None,
            max_requests: Some(2),
        };
        let mut guarded = GuardedProvider::new(&local, ConsentPolicy::local_only(), budget);
        let req = CompletionRequest::new(
            "llama",
            vec![ChatMessage::user("summarize this long passage please")],
        );
        guarded.complete(&req).expect("first fits in budget");
        let err = guarded
            .complete(&req)
            .expect_err("second exceeds 65 token cap");
        assert!(matches!(err, ProviderError::BudgetExceeded { .. }));
        assert_eq!(local.call_count(), 1);
    }

    #[test]
    fn vendor_response_parsers_decode_openai_ollama_anthropic_gemini_and_sse() {
        let openai = OpenAiCompatibleProvider::parse_chat_response(
            r#"{"model":"gpt-4o","choices":[{"message":{"content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
        )
        .expect("openai parses");
        assert_eq!(openai.content, "hello");
        assert_eq!(openai.usage.total_tokens(), 15);

        let sse = OpenAiCompatibleProvider::parse_sse_lines(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\ndata: [DONE]\n",
        );
        assert_eq!(sse.len(), 3);

        let ollama = OllamaProvider::parse_chat_response(
            r#"{"model":"llama3.2","message":{"content":"local ok"},"prompt_eval_count":7,"eval_count":3,"done_reason":"stop"}"#,
        )
        .expect("ollama parses");
        assert_eq!(ollama.content, "local ok");
        assert_eq!(ollama.usage.total_tokens(), 10);

        let anthropic = AnthropicProvider::parse_messages_response(
            r#"{"model":"claude-sonnet","content":[{"type":"text","text":"cited"}],"stop_reason":"end_turn","usage":{"input_tokens":8,"output_tokens":4}}"#,
        )
        .expect("anthropic parses");
        assert_eq!(anthropic.content, "cited");
        assert_eq!(anthropic.usage.total_tokens(), 12);

        let gemini = GeminiProvider::parse_generate_response(
            r#"{"candidates":[{"content":{"parts":[{"text":"gem"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":6,"candidatesTokenCount":2}}"#,
        )
        .expect("gemini parses");
        assert_eq!(gemini.content, "gem");
        assert_eq!(gemini.usage.total_tokens(), 8);
    }
}
