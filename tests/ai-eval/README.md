# AI Evaluation and Safety Suite (`I.12`)

This directory stores the deterministic evaluation and adversarial safety dataset for `excellent-ai` (`tests/ai-eval/corpus.json`).

## Metrics Verified

- **Retrieval `recall@10`**: checks that expected source pages appear in top-10 hybrid BM25 + vector results.
- **Citation accuracy**: verifies every cited claim maps to an indexed `chunk_id` and matches a verbatim (or normalized) quote in that passage.
- **Hallucination rate**: ensures unverified or fabricated claims/citations are rejected and never surfaced as cited.
- **Prompt-injection resistance**: tests XML delimiter escaping (`</document_passage>`) and instruction isolation against adversarial PDFs.
- **Consent boundary**: asserts zero provider invocations occur when AI is disabled or cloud transmission lacks explicit consent.

## Running

```powershell
cargo test -p excellent-ai --test ai_eval
```
