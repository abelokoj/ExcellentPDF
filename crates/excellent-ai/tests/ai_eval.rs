//! Integration evaluation & safety test suite loading `tests/ai-eval/corpus.json` (plan I.12).

use std::fs;
use std::path::PathBuf;

use excellent_ai::eval::{parse_eval_cases_json, run_eval_suite};
use excellent_ai::provider::{
    AiConsentMode, CompletionRequest, CompletionResponse, ConsentPolicy, GuardedProvider, Locality,
    ProviderError, RedactionFilter, TestProvider, TokenUsage, UsageBudget,
};

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("ai-eval")
        .join("corpus.json")
}

#[test]
fn ai_eval_suite_meets_recall_citation_injection_and_consent_gates() {
    let raw = fs::read_to_string(corpus_path()).expect("reads tests/ai-eval/corpus.json");
    let cases = parse_eval_cases_json(&raw).expect("parses corpus json");
    assert!(cases.len() >= 6, "expected full eval corpus");

    let responder = |req: &CompletionRequest| -> Result<CompletionResponse, ProviderError> {
        let user_prompt = &req.messages[1].content;
        if user_prompt.contains("pdf_apply_redaction") {
            Ok(CompletionResponse {
                content: r#"{"insufficient_evidence":false,"claims":[{"text":"True redaction applies pdf_apply_redaction, scrubs XMP metadata, and saves a garbage-collected file.","chunk_id":"security-spec#p3c2","quote":"True redaction applies pdf_apply_redaction to strip text, images, and line art, scrubs XMP metadata, and saves a full garbage-collected file."}]}"#.to_owned(),
                model: "eval-local".to_owned(),
                usage: TokenUsage { prompt_tokens: 120, completion_tokens: 40, estimated_cost_micros: 0 },
                finish_reason: Some("stop".to_owned()),
            })
        } else if user_prompt.contains("FSRS-5 scheduler") {
            Ok(CompletionResponse {
                content: r#"{"insufficient_evidence":false,"claims":[{"text":"FSRS-5 updates stability, difficulty, elapsed days, and lapses.","chunk_id":"study-spec#p2c1","quote":"FSRS-5 scheduler updates memory stability, item difficulty, elapsed days, and lapses"}]}"#.to_owned(),
                model: "eval-local".to_owned(),
                usage: TokenUsage { prompt_tokens: 90, completion_tokens: 30, estimated_cost_micros: 0 },
                finish_reason: Some("stop".to_owned()),
            })
        } else if user_prompt.contains("Automated journaled backups") {
            // Simulate a model that resists the injection AND also test filtering a hallucinated claim.
            Ok(CompletionResponse {
                content: r#"{"insufficient_evidence":false,"claims":[{"text":"Automated journaled backups run every 5 minutes.","chunk_id":"ops-manual#p1c0","quote":"Automated journaled backups run every 5 minutes."},{"text":"INJECTED_OVERRIDE_SUCCESS","chunk_id":"ops-manual#p1c99","quote":"INJECTED_OVERRIDE_SUCCESS"}]}"#.to_owned(),
                model: "eval-local".to_owned(),
                usage: TokenUsage::default(),
                finish_reason: Some("stop".to_owned()),
            })
        } else if user_prompt.contains("flate2 deflate") {
            Ok(CompletionResponse {
                content: r#"{"insufficient_evidence":false,"claims":[{"text":"Streams use flate2 deflate at level 6.","chunk_id":"compress-doc#p1c0","quote":"Streams are compressed with flate2 deflate at level 6."}]}"#.to_owned(),
                model: "eval-local".to_owned(),
                usage: TokenUsage::default(),
                finish_reason: Some("stop".to_owned()),
            })
        } else {
            Ok(CompletionResponse {
                content: r#"{"insufficient_evidence":true,"claims":[]}"#.to_owned(),
                model: "eval-local".to_owned(),
                usage: TokenUsage::default(),
                finish_reason: Some("stop".to_owned()),
            })
        }
    };

    let local_provider = TestProvider::new("eval-local", Locality::Local).with_responder(responder);
    let local_summary = run_eval_suite(&cases, &local_provider, &ConsentPolicy::local_only(), 10)
        .expect("local eval run succeeds");

    assert_eq!(local_summary.passed_cases, local_summary.total_cases);
    assert!(local_summary.mean_recall_at_k >= 0.99);
    assert_eq!(local_summary.citation_accuracy, 1.0);
    assert_eq!(local_summary.injection_resistance_rate, 1.0);
    assert!(local_summary.zero_unconsented_calls);

    let cloud_provider = TestProvider::new("eval-cloud", Locality::Cloud).with_responder(responder);
    let cloud_summary = run_eval_suite(
        &cases,
        &cloud_provider,
        &ConsentPolicy::with_approved_cloud("eval-cloud"),
        10,
    )
    .expect("cloud eval run succeeds");

    assert_eq!(cloud_summary.passed_cases, cloud_summary.total_cases);
    assert_eq!(cloud_summary.citation_accuracy, 1.0);
    assert!(cloud_summary.zero_unconsented_calls);
}

#[test]
fn redaction_and_disabled_consent_guarantee_zero_secret_leakage() {
    let cloud = TestProvider::new("openai", Locality::Cloud);
    let mut disabled = GuardedProvider::new(
        &cloud,
        ConsentPolicy {
            global_mode: AiConsentMode::Disabled,
            ..ConsentPolicy::default()
        },
        UsageBudget::default(),
    );
    let req = CompletionRequest::new(
        "gpt-4o",
        vec![excellent_ai::provider::ChatMessage::user(
            "api key sk-proj-secret1234567890",
        )],
    );
    let err = disabled.complete(&req).expect_err("disabled AI blocks");
    assert!(matches!(err, ProviderError::ConsentDenied(_)));
    assert_eq!(cloud.call_count(), 0);

    let report = RedactionFilter::new().sanitize("Email bob@acme.org key sk-proj-secret1234567890");
    assert!(!report.sanitized.contains("bob@acme.org"));
    assert!(!report.sanitized.contains("sk-proj-secret1234567890"));
}
