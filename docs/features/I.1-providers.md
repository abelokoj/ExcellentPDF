# Ph6: 4/13 | I.1: Providers

**Tag:** I.1; **Priority:** P1; **Phase:** 6; **Needs:** K.21, K.19; **Code:** `crates/excellent-ai/src/provider.rs`
**Plan:** §27 entry `#### Ph6: 4/13 | I.1` and §12.1–§12.3.

## Problem

Readers need optional AI assistance without sacrificing privacy, vendor choice, or offline capability.

## User story

As a reader, I want to pick a local model (Ollama or llama.cpp) or an explicitly consented cloud provider with clear local/cloud visibility and spend caps.

## UI sketch

- Settings -> AI: consent mode (`Disabled`, `LocalOnly`, `LocalAndCloudWithPrompt`, `CloudApproved`), per-provider setup, token/cost budget caps, and redaction toggle.
- Chat panel header and status bar show a visible `Local` or `Cloud` badge plus workspace/document off switches.

## Mode

Available in Read, Annotate, Study, and Notebook modes when enabled; off by default.

## Speed and size impact

- Startup: zero AI code or HTTP connections run at startup; `excellent-ai` initializes lazily on first AI action.
- Installer size: HTTP client (`ureq`) is already shared across the workspace; local model runtimes live in downloadable packs.

## Design

- **Core:** `AiProvider` trait (`complete`, `stream`, `embed`) with `OpenAiCompatibleProvider` (`llama-server`, `OpenAI`, custom endpoints), `OllamaProvider`, `AnthropicProvider`, `GeminiProvider`, and `TestProvider`.
- **Consent & Safety:** `ConsentGate` enforces `AiConsentMode`, `ScopeConsent` (workspace/document off switch), and explicit cloud provider approval. `RedactionFilter` masks API keys, emails, phone numbers, SSNs, payment cards, and custom terms before dispatch. `BudgetTracker` enforces token, request, and micro-dollar caps.

## Acceptance tests

- [x] `consent_gate_blocks_all_calls_when_disabled_or_unapproved_cloud`: disabled or unapproved cloud providers execute zero calls.
- [x] `document_and_workspace_switches_override_global_consent`: document/workspace off switches block dispatch.
- [x] `redaction_filter_masks_secrets_and_pii_before_dispatch`: secrets and PII are masked before reaching providers.
- [x] `budget_tracker_enforces_token_and_request_caps`: token and request limits block over-budget calls.
- [x] `vendor_response_parsers_decode_openai_ollama_anthropic_gemini_and_sse`: local and cloud response formats parse identically.
