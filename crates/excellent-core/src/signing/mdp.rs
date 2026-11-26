//! Signature-safe editing (plan O.16): incremental-update enforcement and DocMDP P1/P2/P3
//! + FieldMDP permission checks to detect edits that would invalidate existing signatures.

use crate::forms::pdf_codec::{PdfIndex, PdfVal};

use super::der::SigningError;
use super::pdf_sig::parse_doc_mdp_from_sig_dict;

/// PDF `/DocMDP` `/P` certification permission level (ISO 32000-1 §12.8.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocMdpPermission {
    /// `/P 1`: No changes to the document are permitted; any change invalidates the signature.
    NoChangesAllowed,
    /// `/P 2`: Permitted changes are filling in forms, instantiating page templates, and signing.
    FormFillAndSign,
    /// `/P 3`: Permitted changes are `/P 2` plus annotation creation, deletion, and modification.
    FormFillSignAndAnnotate,
}

impl DocMdpPermission {
    /// Parses PDF `/TransformParams /P` integer (`1`, `2`, or `3`).
    #[must_use]
    pub const fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::NoChangesAllowed),
            2 => Some(Self::FormFillAndSign),
            3 => Some(Self::FormFillSignAndAnnotate),
            _ => None,
        }
    }

    /// Numeric PDF `/P` code (`1`, `2`, or `3`).
    #[must_use]
    pub const fn code(self) -> i64 {
        match self {
            Self::NoChangesAllowed => 1,
            Self::FormFillAndSign => 2,
            Self::FormFillSignAndAnnotate => 3,
        }
    }
}

/// PDF `/FieldMDP` lock rule attached to a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMdpRule {
    /// All form fields are locked (`/Action /All`).
    All,
    /// Only the named fields are locked (`/Action /Include`).
    Include(Vec<String>),
    /// All fields except the named ones are locked (`/Action /Exclude`).
    Exclude(Vec<String>),
}

impl FieldMdpRule {
    /// Whether `field_name` is locked by this rule.
    #[must_use]
    pub fn locks_field(&self, field_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Include(names) => names.iter().any(|n| n == field_name),
            Self::Exclude(names) => !names.iter().any(|n| n == field_name),
        }
    }
}

/// Classification of a proposed document edit for signature safety evaluation (plan O.16).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DocumentEditKind {
    /// Filling or modifying an interactive form field.
    FormFill {
        /// Optional target field name to check against `/FieldMDP` locks.
        field_name: Option<String>,
    },
    /// Appending an additional digital signature.
    Sign,
    /// Creating, editing, or deleting page annotations.
    Annotation,
    /// Structural page operations (reorder, delete, insert, crop, flatten).
    PageStructure,
    /// Modifying existing page content streams (text edit, image replacement, redaction).
    ContentEdit,
    /// Changing document metadata, encryption, or permissions.
    MetadataOrSecurity,
}

/// Decision returned by [`inspect_edit_safety`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditSafetyVerdict {
    /// Edit is permitted under the active DocMDP / FieldMDP policies when saved incrementally.
    Allowed,
    /// Edit would invalidate one or more existing signatures.
    WouldInvalidate {
        /// Names of existing signatures that would break or be marked non-compliant.
        affected_signatures: Vec<String>,
        /// Human-readable explanation.
        reason: String,
    },
}

/// Full signature-safety inspection report for a proposed edit (plan O.16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSafetyReport {
    /// Most restrictive active DocMDP level if a certification signature is present.
    pub policy: Option<DocMdpPermission>,
    /// Number of existing signatures in the document.
    pub existing_signatures: usize,
    /// Whether saving this document must use incremental updates only.
    pub requires_incremental_save: bool,
    /// Safety verdict for the proposed edit.
    pub verdict: EditSafetyVerdict,
}

/// Inspects `pdf_bytes` for existing signatures, `/DocMDP` certification levels, and `/FieldMDP`
/// rules, and decides whether `edit` is safe to apply incrementally (plan O.16).
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `pdf_bytes` cannot be parsed as a PDF.
pub fn inspect_edit_safety(
    pdf_bytes: &[u8],
    edit: &DocumentEditKind,
) -> Result<EditSafetyReport, SigningError> {
    let index = PdfIndex::parse(pdf_bytes)?;
    let mut sig_names = Vec::new();
    let mut strictest_mdp: Option<DocMdpPermission> = None;
    let mut field_mdp_rules = Vec::new();

    for &obj_id in index.objects.keys() {
        let Ok(val) = index.read_value(obj_id) else {
            continue;
        };
        let Some(dict) = val.as_dict() else {
            continue;
        };
        if dict.get("Type").and_then(PdfVal::as_name) != Some("Sig")
            && !(dict.contains_key("ByteRange") && dict.contains_key("Contents"))
        {
            continue;
        }
        let name = dict
            .get("Name")
            .and_then(PdfVal::as_text)
            .unwrap_or_else(|| format!("Signature_{obj_id}"));
        sig_names.push(name);

        if let Some(mdp) = parse_doc_mdp_from_sig_dict(dict) {
            strictest_mdp = Some(match strictest_mdp {
                Some(prev) => prev.min(mdp),
                None => mdp,
            });
        }
        if let Some(rule) = parse_field_mdp_from_sig_dict(dict) {
            field_mdp_rules.push(rule);
        }
    }

    if sig_names.is_empty() {
        return Ok(EditSafetyReport {
            policy: None,
            existing_signatures: 0,
            requires_incremental_save: false,
            verdict: EditSafetyVerdict::Allowed,
        });
    }

    // Approval signatures without explicit DocMDP default to permitting form fill & co-signing (P2)
    // when saved incrementally, while structural/content edits always break existing signatures.
    let effective_policy = strictest_mdp.unwrap_or(DocMdpPermission::FormFillAndSign);

    let verdict = match edit {
        DocumentEditKind::PageStructure
        | DocumentEditKind::ContentEdit
        | DocumentEditKind::MetadataOrSecurity => EditSafetyVerdict::WouldInvalidate {
            affected_signatures: sig_names.clone(),
            reason: "structural, content, or security edits invalidate existing signatures"
                .to_owned(),
        },
        DocumentEditKind::Annotation => match effective_policy {
            DocMdpPermission::FormFillSignAndAnnotate => EditSafetyVerdict::Allowed,
            DocMdpPermission::FormFillAndSign | DocMdpPermission::NoChangesAllowed => {
                EditSafetyVerdict::WouldInvalidate {
                    affected_signatures: sig_names.clone(),
                    reason: format!(
                        "DocMDP level P{} forbids annotation changes",
                        effective_policy.code()
                    ),
                }
            }
        },
        DocumentEditKind::Sign => match effective_policy {
            DocMdpPermission::NoChangesAllowed => EditSafetyVerdict::WouldInvalidate {
                affected_signatures: sig_names.clone(),
                reason: "DocMDP level P1 forbids adding further signatures".to_owned(),
            },
            DocMdpPermission::FormFillAndSign | DocMdpPermission::FormFillSignAndAnnotate => {
                EditSafetyVerdict::Allowed
            }
        },
        DocumentEditKind::FormFill { field_name } => match effective_policy {
            DocMdpPermission::NoChangesAllowed => EditSafetyVerdict::WouldInvalidate {
                affected_signatures: sig_names.clone(),
                reason: "DocMDP level P1 forbids form filling".to_owned(),
            },
            DocMdpPermission::FormFillAndSign | DocMdpPermission::FormFillSignAndAnnotate => {
                if let Some(target) = field_name
                    && field_mdp_rules.iter().any(|r| r.locks_field(target))
                {
                    EditSafetyVerdict::WouldInvalidate {
                        affected_signatures: sig_names.clone(),
                        reason: format!("field {target:?} is locked by /FieldMDP"),
                    }
                } else {
                    EditSafetyVerdict::Allowed
                }
            }
        },
    };

    Ok(EditSafetyReport {
        policy: strictest_mdp,
        existing_signatures: sig_names.len(),
        requires_incremental_save: true,
        verdict,
    })
}

/// Verifies that `candidate_pdf` preserves `original_signed_pdf` byte-for-byte as its prefix
/// (strict incremental update mode, plan O.16), returning `candidate_pdf` if valid.
///
/// # Errors
///
/// Returns [`SigningError::PdfStructure`] if `candidate_pdf` rewrote or mutated signed bytes.
pub fn enforce_incremental_save(
    original_signed_pdf: &[u8],
    candidate_pdf: &[u8],
) -> Result<Vec<u8>, SigningError> {
    if !candidate_pdf.starts_with(original_signed_pdf) {
        return Err(SigningError::PdfStructure(
            "non-incremental save would rewrite signed bytes and invalidate existing signatures"
                .to_owned(),
        ));
    }
    Ok(candidate_pdf.to_vec())
}

fn parse_field_mdp_from_sig_dict(
    dict: &std::collections::BTreeMap<String, PdfVal>,
) -> Option<FieldMdpRule> {
    let refs = dict.get("Reference").and_then(PdfVal::as_array)?;
    for item in refs {
        let Some(rdict) = item.as_dict() else {
            continue;
        };
        if rdict.get("TransformMethod").and_then(PdfVal::as_name) == Some("FieldMDP")
            && let Some(tp) = rdict.get("TransformParams").and_then(PdfVal::as_dict)
        {
            let action = tp.get("Action").and_then(PdfVal::as_name)?;
            let fields: Vec<String> = tp
                .get("Fields")
                .and_then(PdfVal::as_array)
                .map(|arr| arr.iter().filter_map(PdfVal::as_text).collect())
                .unwrap_or_default();
            return match action {
                "All" => Some(FieldMdpRule::All),
                "Include" => Some(FieldMdpRule::Include(fields)),
                "Exclude" => Some(FieldMdpRule::Exclude(fields)),
                _ => None,
            };
        }
    }
    None
}
