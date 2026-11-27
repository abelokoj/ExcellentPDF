# ADR 0009: Optional, local-first AI

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D9 (plan §3)

## Context

Users' documents are private, and AI features are useful but not core.

## Decision

AI is optional and provider-agnostic, local first (llama.cpp or an Ollama pack); cloud providers only with explicit consent.

## Why

Privacy: nothing leaves the device without the user's consent.

## Consequences

`ecclesia-ai` defines a provider trait; cloud adapters check consent before every request.
