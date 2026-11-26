//! Interactive form-filling session state, field highlighting, tab navigation, and flattened export
//! (plan O.11, O.13).

use std::collections::BTreeSet;
use std::path::Path;

use excellent_core::forms::{
    AcroForm, CalculationReport, FieldValue, FormError, ValidationReport, save_flattened_forms,
};
use excellent_mupdf::{Document, PageRect};

/// Visual highlight tone for interactive form field overlays ("highlight fields", plan O.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightTone {
    /// Standard fillable field.
    Fillable,
    /// Required field whose current value is empty.
    RequiredEmpty,
    /// Field failing validation rules.
    Invalid,
    /// Auto-calculated field.
    Calculated,
    /// Read-only field.
    ReadOnly,
}

/// Overlay rectangle and tone for rendering one form widget highlight.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldHighlight {
    /// Fully qualified field name.
    pub field_name: String,
    /// Zero-based page index.
    pub page: usize,
    /// Widget rectangle in top-down page points.
    pub rect: PageRect,
    /// Semantic highlight tone.
    pub tone: HighlightTone,
}

/// Stateful editing session over an [`AcroForm`].
#[derive(Debug, Clone)]
pub struct FormSession {
    form: AcroForm,
    dirty_fields: BTreeSet<String>,
    active_field: Option<String>,
}

impl FormSession {
    /// Wraps an [`AcroForm`] in an interactive editing session.
    #[must_use]
    pub fn new(form: AcroForm) -> Self {
        let first = form.tab_ordered_fields().first().map(|f| f.name.clone());
        Self {
            form,
            dirty_fields: BTreeSet::new(),
            active_field: first,
        }
    }

    /// Underlying [`AcroForm`].
    #[must_use]
    pub const fn form(&self) -> &AcroForm {
        &self.form
    }

    /// Mutable access to the underlying [`AcroForm`].
    pub fn form_mut(&mut self) -> &mut AcroForm {
        &mut self.form
    }

    /// Currently focused field name.
    #[must_use]
    pub fn active_field(&self) -> Option<&str> {
        self.active_field.as_deref()
    }

    /// Sets the focused field name if it exists in the form.
    pub fn focus_field(&mut self, name: &str) {
        if self.form.field(name).is_some() {
            self.active_field = Some(name.to_owned());
        }
    }

    /// Advances focus to the next field in PDF tab order (wrapping around).
    pub fn next_tab_field(&mut self) -> Option<&str> {
        let ordered = self.form.tab_ordered_fields();
        if ordered.is_empty() {
            return None;
        }
        let next_idx = match &self.active_field {
            Some(cur) => ordered
                .iter()
                .position(|f| &f.name == cur)
                .map_or(0, |idx| (idx + 1) % ordered.len()),
            None => 0,
        };
        let next_name = ordered[next_idx].name.clone();
        self.active_field = Some(next_name);
        self.active_field.as_deref()
    }

    /// Moves focus to the previous field in PDF tab order (wrapping around).
    pub fn previous_tab_field(&mut self) -> Option<&str> {
        let ordered = self.form.tab_ordered_fields();
        if ordered.is_empty() {
            return None;
        }
        let prev_idx = match &self.active_field {
            Some(cur) => ordered
                .iter()
                .position(|f| &f.name == cur)
                .map_or(0, |idx| if idx == 0 { ordered.len() - 1 } else { idx - 1 }),
            None => ordered.len() - 1,
        };
        let prev_name = ordered[prev_idx].name.clone();
        self.active_field = Some(prev_name);
        self.active_field.as_deref()
    }

    /// Updates `name` and recalculates dependent fields, marking modified fields dirty.
    ///
    /// # Errors
    ///
    /// Returns [`FormError`] if the field is missing, read-only, incompatible, or calculation fails.
    pub fn set_field(
        &mut self,
        name: &str,
        value: FieldValue,
    ) -> Result<CalculationReport, FormError> {
        let report = self.form.set_value(name, value)?;
        self.dirty_fields.insert(name.to_owned());
        for updated in &report.updated_fields {
            self.dirty_fields.insert(updated.clone());
        }
        Ok(report)
    }

    /// Resets every non-read-only field back to its default value (`/DV`) and recalculates.
    ///
    /// # Errors
    ///
    /// Returns [`FormError`] if recalculation fails.
    pub fn reset_to_defaults(&mut self) -> Result<CalculationReport, FormError> {
        for field in self.form.fields_mut() {
            if !field.flags.is_read_only() {
                field.value = field.default_value.clone();
                self.dirty_fields.insert(field.name.clone());
            }
        }
        self.form.recalculate()
    }

    /// Validates all fields in the session.
    #[must_use]
    pub fn validate(&self) -> ValidationReport {
        self.form.validate_all()
    }

    /// Names of fields modified in this session.
    #[must_use]
    pub fn dirty_fields(&self) -> &BTreeSet<String> {
        &self.dirty_fields
    }

    /// Computes highlight rectangles for all widgets ("highlight fields", plan O.11).
    #[must_use]
    pub fn highlights(&self) -> Vec<FieldHighlight> {
        let validation = self.form.validate_all();
        let invalid_names: BTreeSet<&str> = validation
            .issues
            .iter()
            .map(|i| i.field_name.as_str())
            .collect();

        let mut out = Vec::new();
        for field in self.form.fields() {
            let tone = if invalid_names.contains(field.name.as_str()) {
                if field.flags.is_required() && field.value.is_empty() {
                    HighlightTone::RequiredEmpty
                } else {
                    HighlightTone::Invalid
                }
            } else if field.calculate_script.is_some() {
                HighlightTone::Calculated
            } else if field.flags.is_read_only() {
                HighlightTone::ReadOnly
            } else if field.flags.is_required() && field.value.is_empty() {
                HighlightTone::RequiredEmpty
            } else {
                HighlightTone::Fillable
            };

            for widget in &field.widgets {
                out.push(FieldHighlight {
                    field_name: field.name.clone(),
                    page: widget.page,
                    rect: widget.rect,
                    tone,
                });
            }
        }
        out
    }

    /// Saves a flattened copy of `doc` to `dest` with form widgets baked into page content (plan O.13).
    ///
    /// # Errors
    ///
    /// Returns [`FormError`] if `doc` is not a PDF or MuPDF fails to bake or write `dest`.
    pub fn save_flattened_copy(
        &self,
        doc: &Document,
        dest: impl AsRef<Path>,
    ) -> Result<(), FormError> {
        save_flattened_forms(doc, dest)
    }
}
