# Ph6: 13/13 | I.12: Evaluation and safety suite

**Tag:** I.12; **Priority:** P1; **Phase:** 6; **Needs:** I.3; **Code:** `crates/ecclesia-ai/src/eval.rs`, `tests/ai-eval/`, `crates/ecclesia-ai/tests/ai_eval.rs`
**Plan:** §27 entry `#### Ph6: 13/13 | I.12` and §12.3.

## Problem

RAG pipelines regress silently on citation accuracy, retrieval recall, prompt injections, and privacy boundaries unless verified by an automated benchmark.

## User story

As a maintainer, I want an automated evaluation and safety suite running on demand and before releases to verify citation grounding, injection resistance, and zero-unconsented network calls.

## UI sketch

- CLI / CI runner (`cargo test -p ecclesia-ai --test ai_eval`) outputting Markdown and structured metrics (`recall@k`, citation accuracy, hallucination rate, injection resistance, zero unconsented calls).

## Mode

Developer / CI verification suite.

## Speed and size impact

- Zero runtime impact on the shipping application binary (`tests/` and `eval` runner).

## Design

- `tests/ai-eval/corpus.json` defines factual citation, adversarial XML/prompt-injection, missing-evidence refusal, and consent-boundary cases.
- `ecclesia_ai::eval::run_eval_suite` indexes fixtures into SQLite, measures `recall@k`, evaluates `RagEngine::ask`, checks for forbidden injected strings, audits zero provider calls when consent is off, and generates Markdown reports (`EvalSummary::to_markdown`).

## Acceptance tests

- [x] `ai_eval_suite_meets_recall_citation_injection_and_consent_gates`: passes all factual, injection, refusal, and consent cases for local and cloud test providers.
- [x] `redaction_and_disabled_consent_guarantee_zero_secret_leakage`: confirms 0 provider calls when AI is off and PII/secrets are redacted.
