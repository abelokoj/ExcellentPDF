//! AcroForm field model, pure-Rust validation, tab ordering, and PDF `/AcroForm` / `/Widget`
//! round-trip serialization (plan O.11).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::io;

use ecclesia_mupdf::{Align, PageRect};

use super::calc::{
    CalcAction, CalcError, CalculationReport, NumberFormatSpec, evaluate_calc_action,
    format_number, parse_af_date_format, parse_af_number_format, parse_af_percent_format,
    parse_af_range_validate, parse_calc_script, parse_lenient_number, topological_calc_order,
};
use super::pdf_codec::{
    IncrementalPdfWriter, PdfCodecError, PdfIndex, PdfVal, encode_pdf_literal, encode_pdf_name,
    format_pdf_val,
};

/// Maximum regex evaluation steps to guarantee linear/bounded runtime on untrusted patterns.
const MAX_REGEX_STEPS: usize = 50_000;

/// Errors produced by reading, editing, validating, or saving PDF forms.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum FormError {
    /// Underlying I/O error.
    Io(io::Error),
    /// MuPDF failure while opening or flattening a PDF.
    Mupdf(ecclesia_mupdf::MuError),
    /// Raw PDF structure error while reading or updating `/AcroForm` objects.
    PdfStructure(String),
    /// Named form field does not exist.
    FieldNotFound(String),
    /// Attempted to modify a read-only form field.
    ReadOnlyField(String),
    /// Value type is incompatible with the field kind.
    IncompatibleValue {
        /// Target field name.
        field: String,
        /// Explanation of the type mismatch.
        reason: String,
    },
    /// Calculation script or DAG evaluation failed.
    Calculation(CalcError),
}

impl fmt::Display for FormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "form I/O failed: {err}"),
            Self::Mupdf(err) => write!(f, "form engine failed: {err}"),
            Self::PdfStructure(msg) => write!(f, "invalid PDF form structure: {msg}"),
            Self::FieldNotFound(name) => write!(f, "form field not found: {name}"),
            Self::ReadOnlyField(name) => write!(f, "form field is read-only: {name}"),
            Self::IncompatibleValue { field, reason } => {
                write!(f, "incompatible value for field {field}: {reason}")
            }
            Self::Calculation(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for FormError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Mupdf(err) => Some(err),
            Self::Calculation(err) => Some(err),
            Self::PdfStructure(_)
            | Self::FieldNotFound(_)
            | Self::ReadOnlyField(_)
            | Self::IncompatibleValue { .. } => None,
        }
    }
}

impl From<io::Error> for FormError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ecclesia_mupdf::MuError> for FormError {
    fn from(err: ecclesia_mupdf::MuError) -> Self {
        Self::Mupdf(err)
    }
}

impl From<PdfCodecError> for FormError {
    fn from(err: PdfCodecError) -> Self {
        Self::PdfStructure(err.0)
    }
}

impl From<CalcError> for FormError {
    fn from(err: CalcError) -> Self {
        Self::Calculation(err)
    }
}

/// PDF `/Ff` bitflags for form fields (ISO 32000-1 §12.7.3.1, §12.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FieldFlags(u32);

impl FieldFlags {
    /// Bit 1: field cannot be changed by the user.
    pub const READ_ONLY: u32 = 1 << 0;
    /// Bit 2: field must have a value before submission.
    pub const REQUIRED: u32 = 1 << 1;
    /// Bit 3: field is excluded from form data export.
    pub const NO_EXPORT: u32 = 1 << 2;
    /// Bit 13 (Tx): text field allows multiple lines.
    pub const MULTILINE: u32 = 1 << 12;
    /// Bit 14 (Tx): text field masks characters as a password.
    pub const PASSWORD: u32 = 1 << 13;
    /// Bit 15 (Btn): radio buttons cannot all be off.
    pub const NO_TOGGLE_TO_OFF: u32 = 1 << 14;
    /// Bit 16 (Btn): button is a radio button group.
    pub const RADIO: u32 = 1 << 15;
    /// Bit 17 (Btn): button is a pushbutton.
    pub const PUSHBUTTON: u32 = 1 << 16;
    /// Bit 18 (Ch): choice field is a combo box (else list box).
    pub const COMBO: u32 = 1 << 17;
    /// Bit 19 (Ch): combo box includes an editable text box.
    pub const EDIT: u32 = 1 << 18;
    /// Bit 22 (Ch): list box allows selecting multiple options.
    pub const MULTI_SELECT: u32 = 1 << 21;
    /// Bit 25 (Tx): text field is divided into `MaxLen` equal character cells.
    pub const COMB: u32 = 1 << 24;

    /// Creates flags from a raw `/Ff` bitmask.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Returns the raw `/Ff` bitmask.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether every bit in `mask` is set.
    #[must_use]
    pub const fn contains(self, mask: u32) -> bool {
        (self.0 & mask) == mask
    }

    /// Returns flags with `mask` added.
    #[must_use]
    pub const fn with(self, mask: u32) -> Self {
        Self(self.0 | mask)
    }

    /// Returns flags with `mask` cleared.
    #[must_use]
    pub const fn without(self, mask: u32) -> Self {
        Self(self.0 & !mask)
    }

    /// Whether the field is read-only.
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        self.contains(Self::READ_ONLY)
    }

    /// Whether the field is required.
    #[must_use]
    pub const fn is_required(self) -> bool {
        self.contains(Self::REQUIRED)
    }

    /// Whether the field is excluded from export.
    #[must_use]
    pub const fn is_no_export(self) -> bool {
        self.contains(Self::NO_EXPORT)
    }
}

/// Choice widget presentation style (`/FT /Ch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChoiceKind {
    /// Drop-down combo box (`/Ff` bit 18 set).
    #[default]
    Combo,
    /// Scrollable list box.
    List,
}

/// One selectable option in a choice field (`/Opt`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChoiceOption {
    /// Export value sent on form export.
    pub export_value: String,
    /// Label displayed in the UI.
    pub display_label: String,
}

impl ChoiceOption {
    /// Creates an option where display label and export value are identical.
    #[must_use]
    pub fn simple(value: impl Into<String>) -> Self {
        let val = value.into();
        Self {
            export_value: val.clone(),
            display_label: val,
        }
    }
}

/// One radio button option belonging to a radio group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RadioOption {
    /// On-state appearance name (e.g. `"Yes"`, `"ChoiceA"`).
    pub on_state: String,
    /// Optional tooltip / caption.
    pub caption: Option<String>,
}

/// Specific PDF form field type and type-specific attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldKind {
    /// Text input (`/FT /Tx`).
    Text {
        /// Maximum character length (`/MaxLen`).
        max_len: Option<usize>,
        /// Whether multiple lines are accepted.
        multiline: bool,
        /// Whether input is masked as a password.
        password: bool,
        /// Whether characters are spaced into `max_len` comb cells.
        comb: bool,
    },
    /// Two-state checkbox (`/FT /Btn` without Radio or Pushbutton flags).
    Checkbox {
        /// Export value when checked (default `"Yes"`).
        export_value: String,
    },
    /// Mutually exclusive radio button group (`/FT /Btn` with Radio flag).
    RadioGroup {
        /// Candidate on-state names.
        options: Vec<RadioOption>,
    },
    /// Drop-down combo or list box (`/FT /Ch`).
    Choice {
        /// Combo vs List.
        kind: ChoiceKind,
        /// Available options from `/Opt`.
        options: Vec<ChoiceOption>,
        /// Whether a combo box allows custom typed entries.
        editable: bool,
        /// Whether multiple selections are permitted.
        multi_select: bool,
    },
    /// Digital signature widget (`/FT /Sig`).
    Signature {
        /// Whether a `/V` signature dictionary is already attached.
        signed: bool,
        /// Signer `/Name` from `/V`, if present.
        signer_name: Option<String>,
    },
    /// Interactive pushbutton (`/FT /Btn` with Pushbutton flag).
    Pushbutton {
        /// Normal caption (`/MK /CA`).
        caption: Option<String>,
    },
}

/// Value stored in a form field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FieldValue {
    /// Empty / unset value.
    #[default]
    None,
    /// Text string (also used for radio group selected state).
    Text(String),
    /// Checkbox boolean state.
    Boolean(bool),
    /// One or more selected choice export values.
    Choice(Vec<String>),
}

impl FieldValue {
    /// Returns the value formatted as a single display/export string.
    #[must_use]
    pub fn as_display_string(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Text(s) => s.clone(),
            Self::Boolean(true) => "Yes".to_owned(),
            Self::Boolean(false) => "Off".to_owned(),
            Self::Choice(items) => items.join(", "),
        }
    }

    /// Whether the value counts as empty for `Required` validation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::Text(s) => s.trim().is_empty(),
            Self::Boolean(b) => !*b,
            Self::Choice(items) => items.is_empty(),
        }
    }
}

/// Validation constraint attached to a form field.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ValidationRule {
    /// Field value must be non-empty.
    Required,
    /// Numeric value must fall within `[min, max]`.
    NumericRange {
        /// Inclusive lower bound.
        min: Option<f64>,
        /// Inclusive upper bound.
        max: Option<f64>,
    },
    /// Character count must fall within `[min, max]`.
    Length {
        /// Minimum character length.
        min: Option<usize>,
        /// Maximum character length.
        max: Option<usize>,
    },
    /// Text value must match the safe regular expression pattern.
    Pattern(String),
    /// Text value must look like a valid email address (`local@domain.tld`).
    Email,
    /// Text value must match the date template (`YYYY-MM-DD`, `MM/DD/YYYY`, or `DD/MM/YYYY`).
    DateFormat(String),
}

/// One validation failure for a specific field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Fully qualified field name.
    pub field_name: String,
    /// Human-readable validation message.
    pub message: String,
}

/// Validation result for a single field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValidation {
    /// Fully qualified field name.
    pub field_name: String,
    /// Issues detected; empty when valid.
    pub issues: Vec<ValidationIssue>,
}

impl FieldValidation {
    /// Whether the field passed every rule.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Aggregated validation report across an [`AcroForm`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationReport {
    /// All validation issues found across all fields.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Whether every field in the form is valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Visual placement and styling of one `/Widget` annotation belonging to a field.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetAppearance {
    /// Zero-based page index where the widget resides.
    pub page: usize,
    /// Widget bounding rectangle in page points (top-down for UI, converted when writing PDF).
    pub rect: PageRect,
    /// PDF `/Rect` in raw PDF user-space coordinates `[llx, lly, urx, ury]`.
    pub pdf_rect: [f32; 4],
    /// Font resource name from `/DA` (e.g. `"Helv"`, `"TiRo"`, `"Cour"`).
    pub font_name: String,
    /// Font size in points (`0.0` means auto-size).
    pub font_size: f32,
    /// RGB text color (`0.0..=1.0`).
    pub text_color: [f32; 3],
    /// Optional RGB border color (`/MK /BC`).
    pub border_color: Option<[f32; 3]>,
    /// Optional RGB background fill color (`/MK /BG`).
    pub fill_color: Option<[f32; 3]>,
    /// Text quadding (`/Q`).
    pub align: Align,
    /// Radio button on-state name (`/AP /N /<state>`), if this widget is one radio button.
    pub on_state: Option<String>,
    /// Explicit tab index within its page if known.
    pub tab_index: Option<u32>,
    /// Underlying indirect object ID of this widget annotation when parsed from a PDF.
    pub(crate) widget_obj_id: Option<(u32, u16)>,
    /// Underlying indirect object ID of the page owning this widget.
    pub(crate) page_obj_id: Option<(u32, u16)>,
}

impl WidgetAppearance {
    /// Creates a widget placement on zero-based `page` with `rect` in top-down page points.
    #[must_use]
    pub fn at(page: usize, rect: PageRect) -> Self {
        Self {
            page,
            rect,
            pdf_rect: [rect.x0, rect.y0, rect.x1, rect.y1],
            ..Self::default()
        }
    }
}

impl Default for WidgetAppearance {
    fn default() -> Self {
        Self {
            page: 0,
            rect: PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 120.0,
                y1: 24.0,
            },
            pdf_rect: [0.0, 0.0, 120.0, 24.0],
            font_name: "Helv".to_owned(),
            font_size: 12.0,
            text_color: [0.0, 0.0, 0.0],
            border_color: Some([0.2, 0.2, 0.2]),
            fill_color: Some([1.0, 1.0, 1.0]),
            align: Align::Left,
            on_state: None,
            tab_index: None,
            widget_obj_id: None,
            page_obj_id: None,
        }
    }
}

/// Complete model of a PDF AcroForm field (potentially spanning multiple widgets).
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    /// Fully qualified hierarchical field name (e.g. `"buyer.address.zip"`).
    pub name: String,
    /// Tooltip / alternate name (`/TU`).
    pub alternate_name: Option<String>,
    /// Export mapping name (`/TM`).
    pub mapping_name: Option<String>,
    /// Field subtype and options.
    pub kind: FieldKind,
    /// Current field value (`/V`).
    pub value: FieldValue,
    /// Default value (`/DV`) used on reset.
    pub default_value: FieldValue,
    /// Field flags (`/Ff`).
    pub flags: FieldFlags,
    /// Widget placements on pages.
    pub widgets: Vec<WidgetAppearance>,
    /// Declarative validation rules (required, range, length, regex, date).
    pub validation: Vec<ValidationRule>,
    /// Format JavaScript (`/AA /F`), e.g. `AFNumber_Format(...)`.
    pub format_script: Option<String>,
    /// Calculation JavaScript (`/AA /C`), e.g. `AFSimple_Calculate(...)`.
    pub calculate_script: Option<String>,
    /// Validation JavaScript (`/AA /V`), e.g. `AFRange_Validate(...)`.
    pub validate_script: Option<String>,
    /// Default appearance string (`/DA`).
    pub default_appearance: Option<String>,
    /// Indirect object ID of the field dictionary when parsed from a PDF.
    pub(crate) field_obj_id: Option<(u32, u16)>,
}

impl FormField {
    /// Creates a new text field with default widget appearance.
    #[must_use]
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        let val = value.into();
        Self {
            name: name.into(),
            alternate_name: None,
            mapping_name: None,
            kind: FieldKind::Text {
                max_len: None,
                multiline: false,
                password: false,
                comb: false,
            },
            value: if val.is_empty() {
                FieldValue::None
            } else {
                FieldValue::Text(val)
            },
            default_value: FieldValue::None,
            flags: FieldFlags::default(),
            widgets: Vec::new(),
            validation: Vec::new(),
            format_script: None,
            calculate_script: None,
            validate_script: None,
            default_appearance: Some("/Helv 12 Tf 0 g".to_owned()),
            field_obj_id: None,
        }
    }

    /// Creates a new checkbox field.
    #[must_use]
    pub fn checkbox(
        name: impl Into<String>,
        export_value: impl Into<String>,
        initial: FieldValue,
    ) -> Self {
        Self {
            name: name.into(),
            alternate_name: None,
            mapping_name: None,
            kind: FieldKind::Checkbox {
                export_value: export_value.into(),
            },
            value: initial,
            default_value: FieldValue::Boolean(false),
            flags: FieldFlags::default(),
            widgets: Vec::new(),
            validation: Vec::new(),
            format_script: None,
            calculate_script: None,
            validate_script: None,
            default_appearance: None,
            field_obj_id: None,
        }
    }

    /// Export name (`/TM` if present, otherwise `name`).
    #[must_use]
    pub fn export_name(&self) -> &str {
        self.mapping_name.as_deref().unwrap_or(&self.name)
    }
}

/// Complete interactive `/AcroForm` model parsed from or written to a PDF.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AcroForm {
    /// Ordered list of fields in the form.
    fields: Vec<FormField>,
    /// Explicit calculation order (`/CO`) of field names.
    calc_order: Vec<String>,
    /// Whether `/NeedAppearances` was set on `/AcroForm`.
    need_appearances: bool,
    /// `/SigFlags` value from `/AcroForm`.
    sig_flags: u32,
    /// Whether the PDF also contains an `/XFA` entry (AcroForm fallback is used; full XFA is unsupported).
    has_xfa: bool,
}

impl AcroForm {
    /// Creates an empty [`AcroForm`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `/AcroForm` fields, widgets, scripts, and tab order from raw PDF bytes.
    ///
    /// # Errors
    ///
    /// Returns [`FormError::PdfStructure`] if the PDF cross-reference or catalog cannot be parsed.
    pub fn from_pdf_bytes(bytes: &[u8]) -> Result<Self, FormError> {
        let index = PdfIndex::parse(bytes)?;
        let (_, _, catalog) = index.catalog()?;
        let Some(acro_val) = catalog.get("AcroForm") else {
            return Ok(Self::new());
        };
        let mut owned_acro = None;
        let acro_dict = index
            .resolve(acro_val, &mut owned_acro)?
            .as_dict()
            .ok_or_else(|| FormError::PdfStructure("/AcroForm is not a dictionary".to_owned()))?;

        let need_appearances = matches!(acro_dict.get("NeedAppearances"), Some(PdfVal::Bool(true)));
        let sig_flags = acro_dict
            .get("SigFlags")
            .and_then(PdfVal::as_int)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let has_xfa = acro_dict.contains_key("XFA");

        let pages = index.collect_pages()?;
        let mut page_lookup: BTreeMap<u32, (usize, [f32; 4], String)> = BTreeMap::new();
        let mut widget_to_page: BTreeMap<u32, usize> = BTreeMap::new();
        let mut page_annot_order: BTreeMap<(usize, u32), u32> = BTreeMap::new();

        for (page_idx, &(page_obj_id, _)) in pages.iter().enumerate() {
            let pval = index.read_value(page_obj_id)?;
            if let Some(pdict) = pval.as_dict() {
                let media_box =
                    read_pdf_rect(pdict.get("CropBox").or_else(|| pdict.get("MediaBox")))
                        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
                let tabs = pdict
                    .get("Tabs")
                    .and_then(PdfVal::as_name)
                    .unwrap_or("R")
                    .to_owned();
                page_lookup.insert(page_obj_id, (page_idx, media_box, tabs));
                if let Some(annots_val) = pdict.get("Annots") {
                    let mut owned_annots = None;
                    if let Some(annots) = index.resolve(annots_val, &mut owned_annots)?.as_array() {
                        for (order_idx, a) in annots.iter().enumerate() {
                            if let Some((aid, _)) = a.as_ref() {
                                widget_to_page.insert(aid, page_idx);
                                page_annot_order.insert(
                                    (page_idx, aid),
                                    u32::try_from(order_idx).unwrap_or(u32::MAX),
                                );
                            }
                        }
                    }
                }
            }
        }

        let mut fields = Vec::new();
        let mut visited = BTreeSet::new();
        let inherited = InheritedAttrs::default();
        let page_ctx = PageLookupContext {
            pages: &pages,
            page_lookup: &page_lookup,
            widget_to_page: &widget_to_page,
            page_annot_order: &page_annot_order,
        };
        if let Some(fields_val) = acro_dict.get("Fields") {
            let mut owned_fields = None;
            if let Some(arr) = index.resolve(fields_val, &mut owned_fields)?.as_array() {
                for entry in arr {
                    if let Some((obj_id, gen_num)) = entry.as_ref() {
                        collect_field_node(
                            &index,
                            (obj_id, gen_num),
                            "",
                            &inherited,
                            &page_ctx,
                            &mut visited,
                            &mut fields,
                        )?;
                    }
                }
            }
        }

        let mut calc_order = Vec::new();
        if let Some(co_val) = acro_dict.get("CO") {
            let mut owned_co = None;
            if let Some(co_arr) = index.resolve(co_val, &mut owned_co)?.as_array() {
                for entry in co_arr {
                    if let Some((oid, _)) = entry.as_ref()
                        && let Some(field) = fields.iter().find(|f| {
                            f.field_obj_id.is_some_and(|(id, _)| id == oid)
                                || f.widgets
                                    .iter()
                                    .any(|w| w.widget_obj_id.is_some_and(|(wid, _)| wid == oid))
                        })
                    {
                        calc_order.push(field.name.clone());
                    }
                }
            }
        }

        Ok(Self {
            fields,
            calc_order,
            need_appearances,
            sig_flags,
            has_xfa,
        })
    }

    /// Writes updated field values and widget appearance streams back to `original_pdf` as an
    /// incremental PDF update so existing signatures and non-form objects remain intact.
    ///
    /// # Errors
    ///
    /// Returns [`FormError::PdfStructure`] if `original_pdf` cannot be parsed or updated.
    pub fn to_pdf_bytes(&self, original_pdf: &[u8]) -> Result<Vec<u8>, FormError> {
        let index = PdfIndex::parse(original_pdf)?;
        let (root_id, root_gen, mut catalog) = index.catalog()?;
        let mut writer = IncrementalPdfWriter::from_index(&index)?;

        let Some(acro_val) = catalog.get("AcroForm").cloned() else {
            return Ok(original_pdf.to_vec());
        };
        let acro_ref = acro_val.as_ref();
        let mut owned_acro = None;
        let mut acro_dict = index
            .resolve(&acro_val, &mut owned_acro)?
            .as_dict()
            .cloned()
            .ok_or_else(|| FormError::PdfStructure("/AcroForm is not a dictionary".to_owned()))?;

        // Ensure standard Helvetica font exists for /DR /Font /Helv.
        let helv_id = writer.alloc_id();
        writer.write_object(
            helv_id,
            0,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        );

        let mut field_by_obj: BTreeMap<u32, &FormField> = BTreeMap::new();
        for field in &self.fields {
            if let Some((oid, _)) = field.field_obj_id {
                field_by_obj.insert(oid, field);
            }
            for w in &field.widgets {
                if let Some((wid, _)) = w.widget_obj_id {
                    field_by_obj.entry(wid).or_insert(field);
                }
            }
        }

        // Update each known field/widget dictionary and rebuild normal appearance `/AP << /N ... >>`.
        for field in &self.fields {
            let pdf_v = field_value_to_pdf(field);
            if let Some((fid, fgen)) = field.field_obj_id {
                let is_merged_widget = field
                    .widgets
                    .first()
                    .and_then(|w| w.widget_obj_id)
                    .is_some_and(|(wid, _)| wid == fid);

                let mut fdict = index
                    .read_value(fid)?
                    .as_dict()
                    .cloned()
                    .unwrap_or_default();
                if let Some(ref v) = pdf_v {
                    fdict.insert("V".to_owned(), v.clone());
                } else {
                    fdict.remove("V");
                }

                if is_merged_widget && let Some(widget) = field.widgets.first() {
                    let ap_id = write_widget_appearance_stream(&mut writer, field, widget, helv_id);
                    let mut ap_dict = BTreeMap::new();
                    if let FieldKind::Checkbox { export_value } = &field.kind {
                        let mut n_dict = BTreeMap::new();
                        let on_ap_id = write_checkbox_state_stream(
                            &mut writer,
                            widget,
                            CheckboxState::Checked,
                        );
                        let off_ap_id = write_checkbox_state_stream(
                            &mut writer,
                            widget,
                            CheckboxState::Unchecked,
                        );
                        n_dict.insert(export_value.clone(), PdfVal::Ref(on_ap_id, 0));
                        n_dict.insert("Off".to_owned(), PdfVal::Ref(off_ap_id, 0));
                        ap_dict.insert("N".to_owned(), PdfVal::Dict(n_dict));
                        let as_state = if matches!(field.value, FieldValue::Boolean(true)) {
                            export_value.as_str()
                        } else {
                            "Off"
                        };
                        fdict.insert("AS".to_owned(), PdfVal::Name(as_state.to_owned()));
                    } else {
                        ap_dict.insert("N".to_owned(), PdfVal::Ref(ap_id, 0));
                    }
                    fdict.insert("AP".to_owned(), PdfVal::Dict(ap_dict));
                }
                writer.write_object(fid, fgen, &format_pdf_val(&PdfVal::Dict(fdict)));
            }

            // Update separate child widgets (e.g. radio buttons or split text widgets).
            for widget in &field.widgets {
                if let Some((wid, wgen)) = widget.widget_obj_id
                    && Some((wid, wgen)) != field.field_obj_id
                {
                    let mut wdict = index
                        .read_value(wid)?
                        .as_dict()
                        .cloned()
                        .unwrap_or_default();
                    match &field.kind {
                        FieldKind::RadioGroup { .. } => {
                            let on_name = widget.on_state.as_deref().unwrap_or("Yes");
                            let is_selected = matches!(
                                &field.value,
                                FieldValue::Text(sel) if sel == on_name
                            );
                            let as_val = if is_selected { on_name } else { "Off" };
                            wdict.insert("AS".to_owned(), PdfVal::Name(as_val.to_owned()));
                            let on_ap_id =
                                write_radio_state_stream(&mut writer, widget, RadioState::Selected);
                            let off_ap_id = write_radio_state_stream(
                                &mut writer,
                                widget,
                                RadioState::Unselected,
                            );
                            let mut n_dict = BTreeMap::new();
                            n_dict.insert(on_name.to_owned(), PdfVal::Ref(on_ap_id, 0));
                            n_dict.insert("Off".to_owned(), PdfVal::Ref(off_ap_id, 0));
                            let mut ap_dict = BTreeMap::new();
                            ap_dict.insert("N".to_owned(), PdfVal::Dict(n_dict));
                            wdict.insert("AP".to_owned(), PdfVal::Dict(ap_dict));
                        }
                        FieldKind::Checkbox { export_value } => {
                            let as_val = if matches!(field.value, FieldValue::Boolean(true)) {
                                export_value.as_str()
                            } else {
                                "Off"
                            };
                            wdict.insert("AS".to_owned(), PdfVal::Name(as_val.to_owned()));
                        }
                        _ => {
                            let ap_id =
                                write_widget_appearance_stream(&mut writer, field, widget, helv_id);
                            let mut ap_dict = BTreeMap::new();
                            ap_dict.insert("N".to_owned(), PdfVal::Ref(ap_id, 0));
                            wdict.insert("AP".to_owned(), PdfVal::Dict(ap_dict));
                        }
                    }
                    writer.write_object(wid, wgen, &format_pdf_val(&PdfVal::Dict(wdict)));
                }
            }
        }

        // Appearances for every widget were explicitly generated above, so viewers render /AP directly.
        acro_dict.insert("NeedAppearances".to_owned(), PdfVal::Bool(false));
        if let Some((acro_id, acro_gen)) = acro_ref {
            writer.write_object(acro_id, acro_gen, &format_pdf_val(&PdfVal::Dict(acro_dict)));
        } else {
            catalog.insert("AcroForm".to_owned(), PdfVal::Dict(acro_dict));
            writer.write_object(root_id, root_gen, &format_pdf_val(&PdfVal::Dict(catalog)));
        }

        Ok(writer.finish())
    }

    /// Adds a field to the form.
    pub fn add_field(&mut self, field: FormField) {
        self.fields.push(field);
    }

    /// All fields in document order.
    #[must_use]
    pub fn fields(&self) -> &[FormField] {
        &self.fields
    }

    /// Mutable access to all fields.
    pub fn fields_mut(&mut self) -> &mut Vec<FormField> {
        &mut self.fields
    }

    /// Looks up a field by its fully qualified name.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&FormField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Looks up a mutable field by its fully qualified name.
    pub fn field_mut(&mut self, name: &str) -> Option<&mut FormField> {
        self.fields.iter_mut().find(|f| f.name == name)
    }

    /// Explicit `/CO` calculation order.
    #[must_use]
    pub fn calc_order(&self) -> &[String] {
        &self.calc_order
    }

    /// Sets the explicit calculation order (`/CO`).
    pub fn set_calc_order(&mut self, order: impl IntoIterator<Item = String>) {
        self.calc_order = order.into_iter().collect();
    }

    /// Whether `/NeedAppearances` is active.
    #[must_use]
    pub const fn need_appearances(&self) -> bool {
        self.need_appearances
    }

    /// `/SigFlags` bitmask.
    #[must_use]
    pub const fn sig_flags(&self) -> u32 {
        self.sig_flags
    }

    /// Whether the form carries an `/XFA` stream (AcroForm fallback is used; full XFA is a known gap).
    #[must_use]
    pub const fn has_xfa(&self) -> bool {
        self.has_xfa
    }

    /// Sets `name`'s value and runs the calculation DAG across dependent fields.
    ///
    /// # Errors
    ///
    /// Returns [`FormError::FieldNotFound`], [`FormError::ReadOnlyField`],
    /// [`FormError::IncompatibleValue`], or [`FormError::Calculation`].
    pub fn set_value(
        &mut self,
        name: &str,
        value: FieldValue,
    ) -> Result<CalculationReport, FormError> {
        let Some(field) = self.fields.iter_mut().find(|f| f.name == name) else {
            return Err(FormError::FieldNotFound(name.to_owned()));
        };
        if field.flags.is_read_only() {
            return Err(FormError::ReadOnlyField(name.to_owned()));
        }
        validate_value_for_kind(name, &field.kind, &value)?;
        field.value = normalized_value_for_kind(&field.kind, value);
        self.recalculate()
    }

    /// Evaluates every calculated field in topological dependency order.
    ///
    /// # Errors
    ///
    /// Returns [`FormError::Calculation`] if a script fails or has a dependency cycle.
    pub fn recalculate(&mut self) -> Result<CalculationReport, FormError> {
        let mut actions: BTreeMap<String, CalcAction> = BTreeMap::new();
        let mut formats: BTreeMap<String, NumberFormatSpec> = BTreeMap::new();
        let mut percent_formats: BTreeMap<String, usize> = BTreeMap::new();

        for field in &self.fields {
            if let Some(script) = &field.calculate_script {
                actions.insert(field.name.clone(), parse_calc_script(script)?);
            }
            if let Some(fmt) = &field.format_script {
                if let Some(spec) = parse_af_number_format(fmt) {
                    formats.insert(field.name.clone(), spec);
                } else if let Some((decimals, _)) = parse_af_percent_format(fmt) {
                    percent_formats.insert(field.name.clone(), decimals);
                }
            }
        }

        if actions.is_empty() {
            return Ok(CalculationReport::default());
        }

        let order = topological_calc_order(&actions, &self.calc_order)?;
        let mut values_map: BTreeMap<String, String> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.value.as_display_string()))
            .collect();

        let mut updated_fields = Vec::new();
        for field_name in &order {
            let action = &actions[field_name];
            let computed = evaluate_calc_action(action, &values_map)?;
            let formatted = if let Some(spec) = formats.get(field_name) {
                format_number(computed, spec)
            } else if let Some(&dec) = percent_formats.get(field_name) {
                format!("{:.prec$}%", computed * 100.0, prec = dec)
            } else if (computed - computed.round()).abs() < 1e-9 {
                format!("{}", computed.round() as i64)
            } else {
                format!("{computed:.4}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned()
            };

            if values_map.get(field_name).map(String::as_str) != Some(formatted.as_str()) {
                updated_fields.push(field_name.clone());
            }
            values_map.insert(field_name.clone(), formatted.clone());
            if let Some(field) = self.fields.iter_mut().find(|f| &f.name == field_name) {
                field.value = FieldValue::Text(formatted);
            }
        }

        Ok(CalculationReport {
            evaluated_order: order,
            updated_fields,
        })
    }

    /// Returns interactive fields ordered for keyboard Tab navigation: sorted by page, then
    /// explicit tab index, then top-to-bottom row / left-to-right column position.
    #[must_use]
    pub fn tab_ordered_fields(&self) -> Vec<&FormField> {
        let mut ordered: Vec<&FormField> = self
            .fields
            .iter()
            .filter(|f| !matches!(f.kind, FieldKind::Pushbutton { .. }) || !f.widgets.is_empty())
            .collect();
        ordered.sort_by(|a, b| {
            let wa = a.widgets.first();
            let wb = b.widgets.first();
            let key_a = widget_sort_key(wa);
            let key_b = widget_sort_key(wb);
            key_a
                .0
                .cmp(&key_b.0)
                .then_with(|| key_a.1.cmp(&key_b.1))
                .then_with(|| key_a.2.cmp(&key_b.2))
                .then_with(|| key_a.3.cmp(&key_b.3))
                .then_with(|| a.name.cmp(&b.name))
        });
        ordered
    }

    /// Validates a single field against its declarative rules and `/AA /V` validation script.
    ///
    /// # Errors
    ///
    /// Returns [`FormError::FieldNotFound`] if `name` does not exist.
    pub fn validate_field(&self, name: &str) -> Result<FieldValidation, FormError> {
        let field = self
            .field(name)
            .ok_or_else(|| FormError::FieldNotFound(name.to_owned()))?;
        Ok(validate_single_field(field))
    }

    /// Validates every field in the form.
    #[must_use]
    pub fn validate_all(&self) -> ValidationReport {
        let mut issues = Vec::new();
        for field in &self.fields {
            issues.extend(validate_single_field(field).issues);
        }
        ValidationReport { issues }
    }
}

fn widget_sort_key(widget: Option<&WidgetAppearance>) -> (usize, u32, i32, i32) {
    match widget {
        Some(w) => {
            // Quantize y into 6pt bands so horizontally aligned fields tab left-to-right.
            let y_band = (w.rect.y0 / 6.0).round() as i32;
            let x_pos = (w.rect.x0 * 10.0).round() as i32;
            (w.page, w.tab_index.unwrap_or(u32::MAX), y_band, x_pos)
        }
        None => (usize::MAX, u32::MAX, i32::MAX, i32::MAX),
    }
}

fn validate_value_for_kind(
    name: &str,
    kind: &FieldKind,
    value: &FieldValue,
) -> Result<(), FormError> {
    match (kind, value) {
        (_, FieldValue::None) => Ok(()),
        (FieldKind::Text { max_len, .. }, FieldValue::Text(text)) => {
            if let Some(limit) = max_len
                && text.chars().count() > *limit
            {
                return Err(FormError::IncompatibleValue {
                    field: name.to_owned(),
                    reason: format!("text length exceeds MaxLen ({limit})"),
                });
            }
            Ok(())
        }
        (FieldKind::Checkbox { .. }, FieldValue::Boolean(_)) => Ok(()),
        (FieldKind::Checkbox { export_value }, FieldValue::Text(s))
            if s == export_value
                || s == "Off"
                || s.eq_ignore_ascii_case("true")
                || s.eq_ignore_ascii_case("false") =>
        {
            Ok(())
        }
        (FieldKind::RadioGroup { options }, FieldValue::Text(sel)) => {
            if options.is_empty() || sel == "Off" || options.iter().any(|o| &o.on_state == sel) {
                Ok(())
            } else {
                Err(FormError::IncompatibleValue {
                    field: name.to_owned(),
                    reason: format!("unknown radio option {sel:?}"),
                })
            }
        }
        (
            FieldKind::Choice {
                options,
                editable,
                multi_select,
                ..
            },
            FieldValue::Choice(selected),
        ) => {
            if !*multi_select && selected.len() > 1 {
                return Err(FormError::IncompatibleValue {
                    field: name.to_owned(),
                    reason: "field does not allow multiple selections".to_owned(),
                });
            }
            if !*editable && !options.is_empty() {
                for item in selected {
                    if !options
                        .iter()
                        .any(|o| &o.export_value == item || &o.display_label == item)
                    {
                        return Err(FormError::IncompatibleValue {
                            field: name.to_owned(),
                            reason: format!("unknown choice option {item:?}"),
                        });
                    }
                }
            }
            Ok(())
        }
        (
            FieldKind::Choice {
                options, editable, ..
            },
            FieldValue::Text(sel),
        ) => {
            if !*editable
                && !options.is_empty()
                && !options
                    .iter()
                    .any(|o| &o.export_value == sel || &o.display_label == sel)
            {
                return Err(FormError::IncompatibleValue {
                    field: name.to_owned(),
                    reason: format!("unknown choice option {sel:?}"),
                });
            }
            Ok(())
        }
        (FieldKind::Signature { .. }, _) => Err(FormError::IncompatibleValue {
            field: name.to_owned(),
            reason: "signature fields must be filled by cryptographic signing".to_owned(),
        }),
        (FieldKind::Pushbutton { .. }, _) => Err(FormError::IncompatibleValue {
            field: name.to_owned(),
            reason: "pushbuttons do not hold a persistent value".to_owned(),
        }),
        _ => Err(FormError::IncompatibleValue {
            field: name.to_owned(),
            reason: "value variant does not match field type".to_owned(),
        }),
    }
}

fn normalized_value_for_kind(kind: &FieldKind, value: FieldValue) -> FieldValue {
    match (kind, value) {
        (FieldKind::Checkbox { export_value }, FieldValue::Text(s)) => {
            let checked = s == *export_value || s.eq_ignore_ascii_case("true") || s == "1";
            FieldValue::Boolean(checked)
        }
        (FieldKind::Choice { .. }, FieldValue::Text(s)) => {
            if s.is_empty() {
                FieldValue::None
            } else {
                FieldValue::Choice(vec![s])
            }
        }
        (_, v) => v,
    }
}

fn validate_single_field(field: &FormField) -> FieldValidation {
    let mut issues = Vec::new();
    let display = field.value.as_display_string();
    let is_empty = field.value.is_empty();

    if field.flags.is_required()
        && is_empty
        && !field
            .validation
            .iter()
            .any(|r| matches!(r, ValidationRule::Required))
    {
        issues.push(ValidationIssue {
            field_name: field.name.clone(),
            message: "required field is empty".to_owned(),
        });
    }

    for rule in &field.validation {
        match rule {
            ValidationRule::Required => {
                if is_empty {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: "required field is empty".to_owned(),
                    });
                }
            }
            ValidationRule::NumericRange { min, max } => {
                if !is_empty {
                    match parse_lenient_number(&display) {
                        Some(val) => {
                            if let Some(lo) = min
                                && val < *lo
                            {
                                issues.push(ValidationIssue {
                                    field_name: field.name.clone(),
                                    message: format!("value {val} is below minimum {lo}"),
                                });
                            }
                            if let Some(hi) = max
                                && val > *hi
                            {
                                issues.push(ValidationIssue {
                                    field_name: field.name.clone(),
                                    message: format!("value {val} exceeds maximum {hi}"),
                                });
                            }
                        }
                        None => issues.push(ValidationIssue {
                            field_name: field.name.clone(),
                            message: "value is not numeric".to_owned(),
                        }),
                    }
                }
            }
            ValidationRule::Length { min, max } => {
                let len = display.chars().count();
                if let Some(lo) = min
                    && len < *lo
                {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: format!("length {len} is shorter than {lo}"),
                    });
                }
                if let Some(hi) = max
                    && len > *hi
                {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: format!("length {len} exceeds {hi}"),
                    });
                }
            }
            ValidationRule::Pattern(pattern) => {
                if !is_empty && !matches_regex(pattern, &display) {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: format!("value does not match pattern {pattern:?}"),
                    });
                }
            }
            ValidationRule::Email => {
                if !is_empty && !is_valid_email(&display) {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: "value is not a valid email address".to_owned(),
                    });
                }
            }
            ValidationRule::DateFormat(fmt) => {
                if !is_empty && !matches_date_format(&display, fmt) {
                    issues.push(ValidationIssue {
                        field_name: field.name.clone(),
                        message: format!("value does not match date format {fmt}"),
                    });
                }
            }
        }
    }

    if !is_empty
        && let Some(vscript) = &field.validate_script
        && let Some((min, max)) = parse_af_range_validate(vscript)
    {
        if let Some(val) = parse_lenient_number(&display) {
            if let Some(lo) = min
                && val < lo
            {
                issues.push(ValidationIssue {
                    field_name: field.name.clone(),
                    message: format!("value {val} is below minimum {lo}"),
                });
            }
            if let Some(hi) = max
                && val > hi
            {
                issues.push(ValidationIssue {
                    field_name: field.name.clone(),
                    message: format!("value {val} exceeds maximum {hi}"),
                });
            }
        } else {
            issues.push(ValidationIssue {
                field_name: field.name.clone(),
                message: "value must be numeric for range validation".to_owned(),
            });
        }
    }

    if !is_empty
        && let Some(fscript) = &field.format_script
        && let Some(fmt) = parse_af_date_format(fscript)
        && !field
            .validation
            .iter()
            .any(|r| matches!(r, ValidationRule::DateFormat(_)))
        && !matches_date_format(&display, &fmt)
    {
        issues.push(ValidationIssue {
            field_name: field.name.clone(),
            message: format!("value does not match date format {fmt}"),
        });
    }

    FieldValidation {
        field_name: field.name.clone(),
        issues,
    }
}

fn is_valid_email(value: &str) -> bool {
    let trimmed = value.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !local.contains(char::is_whitespace)
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && !domain.contains(char::is_whitespace)
}

fn matches_date_format(value: &str, fmt: &str) -> bool {
    let trimmed = value.trim();
    match fmt.to_ascii_lowercase().as_str() {
        "yyyy-mm-dd" => {
            let parts: Vec<&str> = trimmed.split('-').collect();
            parts.len() == 3
                && parts[0].len() == 4
                && parts[1].len() == 2
                && parts[2].len() == 2
                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                && valid_ymd(
                    parts[0].parse().unwrap_or(0),
                    parts[1].parse().unwrap_or(0),
                    parts[2].parse().unwrap_or(0),
                )
        }
        "mm/dd/yyyy" => {
            let parts: Vec<&str> = trimmed.split('/').collect();
            parts.len() == 3
                && parts[0].len() == 2
                && parts[1].len() == 2
                && parts[2].len() == 4
                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                && valid_ymd(
                    parts[2].parse().unwrap_or(0),
                    parts[0].parse().unwrap_or(0),
                    parts[1].parse().unwrap_or(0),
                )
        }
        "dd/mm/yyyy" => {
            let parts: Vec<&str> = trimmed.split('/').collect();
            parts.len() == 3
                && parts[0].len() == 2
                && parts[1].len() == 2
                && parts[2].len() == 4
                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                && valid_ymd(
                    parts[2].parse().unwrap_or(0),
                    parts[1].parse().unwrap_or(0),
                    parts[0].parse().unwrap_or(0),
                )
        }
        _ => !trimmed.is_empty(),
    }
}

const fn valid_ymd(year: u32, month: u32, day: u32) -> bool {
    if year == 0 || month < 1 || month > 12 || day < 1 {
        return false;
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap =
                (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
            if leap { 29 } else { 28 }
        }
        _ => 0,
    };
    day <= max_day
}

// ---- Bounded pure-Rust regular expression matcher (anchors, classes, quantifiers, groups, alternation) ----

#[derive(Debug, Clone)]
enum RegexAtom {
    Literal(char),
    Any,
    Digit,
    Word,
    Space,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Group(Vec<Vec<RegexPiece>>),
}

#[derive(Debug, Clone)]
struct RegexPiece {
    atom: RegexAtom,
    min: usize,
    max: Option<usize>,
}

/// Evaluates a bounded subset of regular expressions (`^`, `$`, `.`, `\d`, `\w`, `\s`, `[a-z]`,
/// `[^...]`, `*`, `+`, `?`, `{m,n}`, `(...)`, `|`).
#[must_use]
pub fn matches_regex(pattern: &str, text: &str) -> bool {
    let mut pat = pattern;
    let anchor_start = pat.starts_with('^');
    if anchor_start {
        pat = &pat[1..];
    }
    let anchor_end = pat.ends_with('$') && !pat.ends_with("\\$");
    if anchor_end {
        pat = &pat[..pat.len() - 1];
    }
    let pat_chars: Vec<char> = pat.chars().collect();
    let Some((branches, consumed)) = parse_regex_branches(&pat_chars) else {
        return false;
    };
    if consumed != pat_chars.len() {
        return false;
    }
    let input: Vec<char> = text.chars().collect();
    let mut steps = 0usize;
    let starts: Vec<usize> = if anchor_start {
        vec![0]
    } else {
        (0..=input.len()).collect()
    };
    for start in starts {
        for branch in &branches {
            if let Some(end) = match_seq(branch, &input, start, &mut steps)
                && (!anchor_end || end == input.len())
            {
                return true;
            }
        }
    }
    false
}

fn parse_regex_branches(chars: &[char]) -> Option<(Vec<Vec<RegexPiece>>, usize)> {
    let mut branches = Vec::new();
    let mut cur = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            ')' => break,
            '|' => {
                branches.push(cur);
                cur = Vec::new();
                i += 1;
                continue;
            }
            '(' => {
                let (inner, len) = parse_regex_branches(&chars[i + 1..])?;
                i += 1 + len;
                if chars.get(i) != Some(&')') {
                    return None;
                }
                i += 1;
                let (min, max, qlen) = parse_quantifier(&chars[i..]);
                i += qlen;
                cur.push(RegexPiece {
                    atom: RegexAtom::Group(inner),
                    min,
                    max,
                });
            }
            '[' => {
                let (atom, len) = parse_char_class(&chars[i + 1..])?;
                i += 1 + len;
                let (min, max, qlen) = parse_quantifier(&chars[i..]);
                i += qlen;
                cur.push(RegexPiece { atom, min, max });
            }
            '\\' => {
                let esc = *chars.get(i + 1)?;
                i += 2;
                let atom = match esc {
                    'd' => RegexAtom::Digit,
                    'w' => RegexAtom::Word,
                    's' => RegexAtom::Space,
                    other => RegexAtom::Literal(other),
                };
                let (min, max, qlen) = parse_quantifier(&chars[i..]);
                i += qlen;
                cur.push(RegexPiece { atom, min, max });
            }
            '.' => {
                i += 1;
                let (min, max, qlen) = parse_quantifier(&chars[i..]);
                i += qlen;
                cur.push(RegexPiece {
                    atom: RegexAtom::Any,
                    min,
                    max,
                });
            }
            ch => {
                i += 1;
                let (min, max, qlen) = parse_quantifier(&chars[i..]);
                i += qlen;
                cur.push(RegexPiece {
                    atom: RegexAtom::Literal(ch),
                    min,
                    max,
                });
            }
        }
    }
    branches.push(cur);
    Some((branches, i))
}

fn parse_char_class(chars: &[char]) -> Option<(RegexAtom, usize)> {
    let mut i = 0usize;
    let negated = chars.first() == Some(&'^');
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    while i < chars.len() {
        if chars[i] == ']' {
            return Some((RegexAtom::Class { negated, ranges }, i + 1));
        }
        let start = if chars[i] == '\\' {
            let c = *chars.get(i + 1)?;
            i += 2;
            c
        } else {
            let c = chars[i];
            i += 1;
            c
        };
        if chars.get(i) == Some(&'-')
            && let Some(&end) = chars.get(i + 1)
            && end != ']'
        {
            i += 2;
            ranges.push((start, end));
        } else {
            ranges.push((start, start));
        }
    }
    None
}

fn parse_quantifier(chars: &[char]) -> (usize, Option<usize>, usize) {
    match chars.first() {
        Some('*') => (0, None, 1),
        Some('+') => (1, None, 1),
        Some('?') => (0, Some(1), 1),
        Some('{') => {
            if let Some(close) = chars.iter().position(|&c| c == '}') {
                let body: String = chars[1..close].iter().collect();
                if let Some((min_s, max_s)) = body.split_once(',') {
                    if let Ok(min) = min_s.trim().parse::<usize>() {
                        let max = if max_s.trim().is_empty() {
                            None
                        } else {
                            max_s.trim().parse::<usize>().ok()
                        };
                        return (min, max, close + 1);
                    }
                } else if let Ok(exact) = body.trim().parse::<usize>() {
                    return (exact, Some(exact), close + 1);
                }
            }
            (1, Some(1), 0)
        }
        _ => (1, Some(1), 0),
    }
}

fn match_seq(seq: &[RegexPiece], input: &[char], pos: usize, steps: &mut usize) -> Option<usize> {
    if *steps >= MAX_REGEX_STEPS {
        return None;
    }
    *steps += 1;
    let Some((first, rest)) = seq.split_first() else {
        return Some(pos);
    };
    let mut positions = vec![pos];
    let limit = first.max.unwrap_or(input.len().saturating_sub(pos));
    for count in 1..=limit {
        let mut next_positions = Vec::new();
        for &p in &positions {
            if let Some(next_p) = match_atom(&first.atom, input, p, steps) {
                next_positions.push(next_p);
            }
        }
        if next_positions.is_empty() {
            break;
        }
        if count >= first.min {
            for &np in next_positions.iter().rev() {
                if let Some(final_p) = match_seq(rest, input, np, steps) {
                    return Some(final_p);
                }
            }
        }
        positions = next_positions;
        if positions.iter().all(|&p| p >= input.len()) && rest.is_empty() && count >= first.min {
            return positions.first().copied();
        }
    }
    if first.min == 0 {
        return match_seq(rest, input, pos, steps);
    }
    None
}

fn match_atom(atom: &RegexAtom, input: &[char], pos: usize, steps: &mut usize) -> Option<usize> {
    match atom {
        RegexAtom::Group(branches) => {
            for branch in branches {
                if let Some(end) = match_seq(branch, input, pos, steps) {
                    return Some(end);
                }
            }
            None
        }
        _ => {
            let &ch = input.get(pos)?;
            let matched = match atom {
                RegexAtom::Literal(c) => ch == *c,
                RegexAtom::Any => ch != '\n',
                RegexAtom::Digit => ch.is_ascii_digit(),
                RegexAtom::Word => ch.is_ascii_alphanumeric() || ch == '_',
                RegexAtom::Space => ch.is_whitespace(),
                RegexAtom::Class { negated, ranges } => {
                    let hit = ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi);
                    if *negated { !hit } else { hit }
                }
                RegexAtom::Group(_) => false,
            };
            matched.then_some(pos + 1)
        }
    }
}

// ---- PDF /AcroForm parser & appearance stream builder helpers ----

#[derive(Default, Clone)]
struct InheritedAttrs {
    ft: Option<String>,
    ff: Option<u32>,
    v: Option<PdfVal>,
    dv: Option<PdfVal>,
    da: Option<String>,
    q: Option<i64>,
    opt: Option<PdfVal>,
    max_len: Option<usize>,
}

struct PageLookupContext<'a> {
    pages: &'a [(u32, u16)],
    page_lookup: &'a BTreeMap<u32, (usize, [f32; 4], String)>,
    widget_to_page: &'a BTreeMap<u32, usize>,
    page_annot_order: &'a BTreeMap<(usize, u32), u32>,
}

fn collect_field_node(
    index: &PdfIndex<'_>,
    obj_ref: (u32, u16),
    prefix: &str,
    inherited: &InheritedAttrs,
    page_ctx: &PageLookupContext<'_>,
    visited: &mut BTreeSet<u32>,
    out: &mut Vec<FormField>,
) -> Result<(), FormError> {
    if !visited.insert(obj_ref.0) {
        return Ok(());
    }
    let val = index.read_value(obj_ref.0)?;
    let Some(dict) = val.as_dict() else {
        return Ok(());
    };

    let partial_name = dict.get("T").and_then(PdfVal::as_text).unwrap_or_default();
    let full_name = if prefix.is_empty() {
        partial_name
    } else if partial_name.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}.{partial_name}")
    };

    let mut next_inherited = inherited.clone();
    if let Some(ft) = dict.get("FT").and_then(PdfVal::as_name) {
        next_inherited.ft = Some(ft.to_owned());
    }
    if let Some(ff) = dict.get("Ff").and_then(PdfVal::as_int)
        && let Ok(bits) = u32::try_from(ff)
    {
        next_inherited.ff = Some(bits);
    }
    if let Some(v) = dict.get("V") {
        next_inherited.v = Some(v.clone());
    }
    if let Some(dv) = dict.get("DV") {
        next_inherited.dv = Some(dv.clone());
    }
    if let Some(da) = dict.get("DA").and_then(PdfVal::as_text) {
        next_inherited.da = Some(da);
    }
    if let Some(q) = dict.get("Q").and_then(PdfVal::as_int) {
        next_inherited.q = Some(q);
    }
    if let Some(opt) = dict.get("Opt") {
        next_inherited.opt = Some(opt.clone());
    }
    if let Some(ml) = dict.get("MaxLen").and_then(PdfVal::as_int)
        && let Ok(ml_u) = usize::try_from(ml)
    {
        next_inherited.max_len = Some(ml_u);
    }

    // Check if /Kids are child fields (have /T) or pure widget annotations (no /T).
    let kids_refs: Vec<(u32, u16)> = if let Some(kids_val) = dict.get("Kids") {
        let mut owned_kids = None;
        index
            .resolve(kids_val, &mut owned_kids)?
            .as_array()
            .map(|arr| arr.iter().filter_map(PdfVal::as_ref).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut has_subfields = false;
    for &(kid_id, _) in &kids_refs {
        if let Ok(kval) = index.read_value(kid_id)
            && let Some(kdict) = kval.as_dict()
            && kdict.contains_key("T")
        {
            has_subfields = true;
            break;
        }
    }

    if has_subfields {
        for kid in kids_refs {
            collect_field_node(
                index,
                kid,
                &full_name,
                &next_inherited,
                page_ctx,
                visited,
                out,
            )?;
        }
        return Ok(());
    }

    // Leaf field (either merged widget or parent field with pure-widget /Kids).
    let Some(ft) = next_inherited.ft.as_deref() else {
        return Ok(());
    };
    let flags = FieldFlags::from_bits(next_inherited.ff.unwrap_or(0));

    let mut widgets = Vec::new();
    if (dict.get("Subtype").and_then(PdfVal::as_name) == Some("Widget")
        || dict.contains_key("Rect"))
        && let Some(w) = parse_widget_dict(obj_ref, dict, &next_inherited, page_ctx)
    {
        widgets.push(w);
    }
    for kid in kids_refs {
        if let Ok(kval) = index.read_value(kid.0)
            && let Some(kdict) = kval.as_dict()
            && let Some(w) = parse_widget_dict(kid, kdict, &next_inherited, page_ctx)
        {
            widgets.push(w);
        }
    }

    let (kind, value, default_value) =
        build_field_kind_and_values(index, ft, flags, &next_inherited, &widgets, dict)?;

    let AaScripts {
        format: format_script,
        calculate: calculate_script,
        validate: validate_script,
    } = parse_aa_scripts(index, dict)?;
    let mut validation = Vec::new();
    if flags.is_required() {
        validation.push(ValidationRule::Required);
    }
    if let Some(ref vscript) = validate_script
        && let Some((min, max)) = parse_af_range_validate(vscript)
    {
        validation.push(ValidationRule::NumericRange { min, max });
    }
    if let Some(ref fscript) = format_script
        && let Some(date_fmt) = parse_af_date_format(fscript)
    {
        validation.push(ValidationRule::DateFormat(date_fmt));
    }

    let field_name = if full_name.is_empty() {
        format!("field_{}", obj_ref.0)
    } else {
        full_name
    };

    out.push(FormField {
        name: field_name,
        alternate_name: dict.get("TU").and_then(PdfVal::as_text),
        mapping_name: dict.get("TM").and_then(PdfVal::as_text),
        kind,
        value,
        default_value,
        flags,
        widgets,
        validation,
        format_script,
        calculate_script,
        validate_script,
        default_appearance: next_inherited.da,
        field_obj_id: Some(obj_ref),
    });
    Ok(())
}

fn parse_widget_dict(
    widget_ref: (u32, u16),
    dict: &BTreeMap<String, PdfVal>,
    inherited: &InheritedAttrs,
    page_ctx: &PageLookupContext<'_>,
) -> Option<WidgetAppearance> {
    let pdf_rect = read_pdf_rect(dict.get("Rect")).unwrap_or([0.0, 0.0, 120.0, 24.0]);
    let page_idx = dict
        .get("P")
        .and_then(PdfVal::as_ref)
        .and_then(|(pid, _)| page_ctx.page_lookup.get(&pid).map(|&(idx, _, _)| idx))
        .or_else(|| page_ctx.widget_to_page.get(&widget_ref.0).copied())
        .unwrap_or(0);
    let page_obj_id = page_ctx.pages.get(page_idx).copied();
    let media_box = page_obj_id
        .and_then(|(pid, _)| page_ctx.page_lookup.get(&pid).map(|(_, mb, _)| *mb))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rect = PageRect {
        x0: pdf_rect[0].min(pdf_rect[2]) - media_box[0],
        y0: media_box[3] - pdf_rect[1].max(pdf_rect[3]),
        x1: pdf_rect[0].max(pdf_rect[2]) - media_box[0],
        y1: media_box[3] - pdf_rect[1].min(pdf_rect[3]),
    };

    let da = dict
        .get("DA")
        .and_then(PdfVal::as_text)
        .or_else(|| inherited.da.clone())
        .unwrap_or_else(|| "/Helv 12 Tf 0 g".to_owned());
    let (font_name, font_size, text_color) = parse_da_string(&da);
    let align = match dict
        .get("Q")
        .and_then(PdfVal::as_int)
        .or(inherited.q)
        .unwrap_or(0)
    {
        1 => Align::Center,
        2 => Align::Right,
        _ => Align::Left,
    };

    let mk = dict.get("MK").and_then(PdfVal::as_dict);
    let border_color = mk.and_then(|d| parse_rgb_array(d.get("BC")));
    let fill_color = mk.and_then(|d| parse_rgb_array(d.get("BG")));

    let on_state = dict
        .get("AP")
        .and_then(PdfVal::as_dict)
        .and_then(|ap| ap.get("N"))
        .and_then(PdfVal::as_dict)
        .and_then(|n| n.keys().find(|k| k.as_str() != "Off").cloned());

    let tab_index = page_ctx
        .page_annot_order
        .get(&(page_idx, widget_ref.0))
        .copied();

    Some(WidgetAppearance {
        page: page_idx,
        rect,
        pdf_rect,
        font_name,
        font_size,
        text_color,
        border_color,
        fill_color,
        align,
        on_state,
        tab_index,
        widget_obj_id: Some(widget_ref),
        page_obj_id,
    })
}

fn build_field_kind_and_values(
    index: &PdfIndex<'_>,
    ft: &str,
    flags: FieldFlags,
    inherited: &InheritedAttrs,
    widgets: &[WidgetAppearance],
    dict: &BTreeMap<String, PdfVal>,
) -> Result<(FieldKind, FieldValue, FieldValue), FormError> {
    match ft {
        "Tx" => {
            let kind = FieldKind::Text {
                max_len: inherited.max_len,
                multiline: flags.contains(FieldFlags::MULTILINE),
                password: flags.contains(FieldFlags::PASSWORD),
                comb: flags.contains(FieldFlags::COMB),
            };
            let val =
                inherited
                    .v
                    .as_ref()
                    .and_then(PdfVal::as_text)
                    .map_or(FieldValue::None, |s| {
                        if s.is_empty() {
                            FieldValue::None
                        } else {
                            FieldValue::Text(s)
                        }
                    });
            let dv = inherited
                .dv
                .as_ref()
                .and_then(PdfVal::as_text)
                .map_or(FieldValue::None, FieldValue::Text);
            Ok((kind, val, dv))
        }
        "Btn" => {
            if flags.contains(FieldFlags::PUSHBUTTON) {
                let caption = dict
                    .get("MK")
                    .and_then(PdfVal::as_dict)
                    .and_then(|mk| mk.get("CA"))
                    .and_then(PdfVal::as_text);
                Ok((
                    FieldKind::Pushbutton { caption },
                    FieldValue::None,
                    FieldValue::None,
                ))
            } else if flags.contains(FieldFlags::RADIO) {
                let mut options = Vec::new();
                for w in widgets {
                    if let Some(ref on) = w.on_state {
                        options.push(RadioOption {
                            on_state: on.clone(),
                            caption: None,
                        });
                    }
                }
                let val = inherited
                    .v
                    .as_ref()
                    .and_then(|v| v.as_name().map(str::to_owned).or_else(|| v.as_text()))
                    .filter(|s| s != "Off")
                    .map_or(FieldValue::None, FieldValue::Text);
                let dv = inherited
                    .dv
                    .as_ref()
                    .and_then(|v| v.as_name().map(str::to_owned).or_else(|| v.as_text()))
                    .filter(|s| s != "Off")
                    .map_or(FieldValue::None, FieldValue::Text);
                Ok((FieldKind::RadioGroup { options }, val, dv))
            } else {
                let export_value = widgets
                    .first()
                    .and_then(|w| w.on_state.clone())
                    .unwrap_or_else(|| "Yes".to_owned());
                let checked = inherited
                    .v
                    .as_ref()
                    .and_then(|v| v.as_name().map(str::to_owned).or_else(|| v.as_text()))
                    .is_some_and(|s| s != "Off" && !s.is_empty());
                let default_checked = inherited
                    .dv
                    .as_ref()
                    .and_then(|v| v.as_name().map(str::to_owned).or_else(|| v.as_text()))
                    .is_some_and(|s| s != "Off" && !s.is_empty());
                Ok((
                    FieldKind::Checkbox { export_value },
                    FieldValue::Boolean(checked),
                    FieldValue::Boolean(default_checked),
                ))
            }
        }
        "Ch" => {
            let kind_style = if flags.contains(FieldFlags::COMBO) {
                ChoiceKind::Combo
            } else {
                ChoiceKind::List
            };
            let mut options = Vec::new();
            if let Some(opt_val) = &inherited.opt {
                let mut owned_opt = None;
                if let Some(arr) = index.resolve(opt_val, &mut owned_opt)?.as_array() {
                    for item in arr {
                        match item {
                            PdfVal::Str(_) => {
                                if let Some(t) = item.as_text() {
                                    options.push(ChoiceOption::simple(t));
                                }
                            }
                            PdfVal::Array(pair) if !pair.is_empty() => {
                                let exp = pair[0].as_text().unwrap_or_default();
                                let disp = pair
                                    .get(1)
                                    .and_then(PdfVal::as_text)
                                    .unwrap_or_else(|| exp.clone());
                                options.push(ChoiceOption {
                                    export_value: exp,
                                    display_label: disp,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            let val = parse_choice_value(inherited.v.as_ref());
            let dv = parse_choice_value(inherited.dv.as_ref());
            Ok((
                FieldKind::Choice {
                    kind: kind_style,
                    options,
                    editable: flags.contains(FieldFlags::EDIT),
                    multi_select: flags.contains(FieldFlags::MULTI_SELECT),
                },
                val,
                dv,
            ))
        }
        "Sig" => {
            let mut signed = false;
            let mut signer_name = None;
            if let Some(v) = &inherited.v {
                let mut owned_v = None;
                if let Some(sig_dict) = index.resolve(v, &mut owned_v)?.as_dict() {
                    signed =
                        sig_dict.contains_key("ByteRange") && sig_dict.contains_key("Contents");
                    signer_name = sig_dict.get("Name").and_then(PdfVal::as_text);
                }
            }
            Ok((
                FieldKind::Signature {
                    signed,
                    signer_name,
                },
                FieldValue::None,
                FieldValue::None,
            ))
        }
        other => Err(FormError::PdfStructure(format!(
            "unsupported form field type /{other}"
        ))),
    }
}

fn parse_choice_value(val: Option<&PdfVal>) -> FieldValue {
    match val {
        None | Some(PdfVal::Null) => FieldValue::None,
        Some(PdfVal::Array(arr)) => {
            let items: Vec<String> = arr.iter().filter_map(PdfVal::as_text).collect();
            if items.is_empty() {
                FieldValue::None
            } else {
                FieldValue::Choice(items)
            }
        }
        Some(other) => other
            .as_text()
            .filter(|s| !s.is_empty())
            .map_or(FieldValue::None, |s| FieldValue::Choice(vec![s])),
    }
}

#[derive(Default)]
struct AaScripts {
    format: Option<String>,
    calculate: Option<String>,
    validate: Option<String>,
}

fn parse_aa_scripts(
    index: &PdfIndex<'_>,
    dict: &BTreeMap<String, PdfVal>,
) -> Result<AaScripts, FormError> {
    let Some(aa_val) = dict.get("AA") else {
        return Ok(AaScripts::default());
    };
    let mut owned_aa = None;
    let Some(aa_dict) = index.resolve(aa_val, &mut owned_aa)?.as_dict() else {
        return Ok(AaScripts::default());
    };
    Ok(AaScripts {
        format: read_js_action(index, aa_dict.get("F"))?,
        calculate: read_js_action(index, aa_dict.get("C"))?,
        validate: read_js_action(index, aa_dict.get("V"))?,
    })
}

fn read_js_action(
    index: &PdfIndex<'_>,
    action: Option<&PdfVal>,
) -> Result<Option<String>, FormError> {
    let Some(act_val) = action else {
        return Ok(None);
    };
    let mut owned = None;
    let Some(act_dict) = index.resolve(act_val, &mut owned)?.as_dict() else {
        return Ok(None);
    };
    if let Some(js_val) = act_dict.get("JS") {
        if let Some(text) = js_val.as_text() {
            return Ok(Some(text));
        }
        if let Some((oid, _)) = js_val.as_ref()
            && let Ok((_, bytes)) = index.read_stream(oid)
        {
            return Ok(Some(super::pdf_codec::decode_pdf_text(&bytes)));
        }
    }
    Ok(None)
}

fn read_pdf_rect(val: Option<&PdfVal>) -> Option<[f32; 4]> {
    let arr = val?.as_array()?;
    if arr.len() < 4 {
        return None;
    }
    Some([
        arr[0].as_f64()? as f32,
        arr[1].as_f64()? as f32,
        arr[2].as_f64()? as f32,
        arr[3].as_f64()? as f32,
    ])
}

fn parse_rgb_array(val: Option<&PdfVal>) -> Option<[f32; 3]> {
    let arr = val?.as_array()?;
    match arr.len() {
        1 => {
            let g = arr[0].as_f64()? as f32;
            Some([g, g, g])
        }
        3 => Some([
            arr[0].as_f64()? as f32,
            arr[1].as_f64()? as f32,
            arr[2].as_f64()? as f32,
        ]),
        _ => None,
    }
}

fn parse_da_string(da: &str) -> (String, f32, [f32; 3]) {
    let mut font_name = "Helv".to_owned();
    let mut font_size = 12.0f32;
    let mut color = [0.0f32; 3];
    let tokens: Vec<&str> = da.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if *tok == "Tf" && i >= 2 {
            font_name = tokens[i - 2].trim_start_matches('/').to_owned();
            if let Ok(sz) = tokens[i - 1].parse::<f32>()
                && sz > 0.0
            {
                font_size = sz;
            }
        } else if *tok == "g" && i >= 1 {
            if let Ok(g) = tokens[i - 1].parse::<f32>() {
                color = [g, g, g];
            }
        } else if *tok == "rg"
            && i >= 3
            && let (Ok(r), Ok(g), Ok(b)) = (
                tokens[i - 3].parse::<f32>(),
                tokens[i - 2].parse::<f32>(),
                tokens[i - 1].parse::<f32>(),
            )
        {
            color = [r, g, b];
        }
    }
    (font_name, font_size, color)
}

fn field_value_to_pdf(field: &FormField) -> Option<PdfVal> {
    match (&field.kind, &field.value) {
        (FieldKind::Signature { .. } | FieldKind::Pushbutton { .. }, _) => None,
        (FieldKind::Checkbox { export_value }, FieldValue::Boolean(checked)) => {
            Some(PdfVal::Name(if *checked {
                export_value.clone()
            } else {
                "Off".to_owned()
            }))
        }
        (FieldKind::RadioGroup { .. }, FieldValue::Text(sel)) => Some(PdfVal::Name(sel.clone())),
        (FieldKind::Choice { .. }, FieldValue::Choice(items)) => {
            if items.is_empty() {
                None
            } else if items.len() == 1 {
                Some(PdfVal::Str(items[0].as_bytes().to_vec()))
            } else {
                Some(PdfVal::Array(
                    items
                        .iter()
                        .map(|s| PdfVal::Str(s.as_bytes().to_vec()))
                        .collect(),
                ))
            }
        }
        (_, FieldValue::None) => None,
        (_, FieldValue::Text(s)) => Some(PdfVal::Str(s.as_bytes().to_vec())),
        (_, FieldValue::Boolean(b)) => {
            Some(PdfVal::Name(if *b { "Yes" } else { "Off" }.to_owned()))
        }
        (_, FieldValue::Choice(items)) => Some(PdfVal::Str(items.join(", ").into_bytes())),
    }
}

fn write_widget_appearance_stream(
    writer: &mut IncrementalPdfWriter,
    field: &FormField,
    widget: &WidgetAppearance,
    font_id: u32,
) -> u32 {
    let w = (widget.pdf_rect[2] - widget.pdf_rect[0]).abs().max(10.0);
    let h = (widget.pdf_rect[3] - widget.pdf_rect[1]).abs().max(10.0);
    let display = match &field.kind {
        FieldKind::Text { password: true, .. } => {
            "*".repeat(field.value.as_display_string().chars().count())
        }
        _ => field.value.as_display_string(),
    };
    let font_size = if widget.font_size > 0.0 {
        widget.font_size
    } else {
        (h * 0.65).clamp(8.0, 14.0)
    };
    let baseline_y = ((h - font_size) * 0.5 + 2.0).max(2.0);
    let [r, g, b] = widget.text_color;
    let mut ops = String::new();
    if let Some([fr, fg, fb]) = widget.fill_color {
        let _ = writeln!(ops, "{fr:.3} {fg:.3} {fb:.3} rg 0 0 {w:.2} {h:.2} re f");
    }
    if let Some([br, bg, bb]) = widget.border_color {
        let _ = writeln!(
            ops,
            "{br:.3} {bg:.3} {bb:.3} RG 1 w 0.5 0.5 {:.2} {:.2} re S",
            w - 1.0,
            h - 1.0
        );
    }
    if !display.is_empty() {
        let font_res = encode_pdf_name(&widget.font_name);
        let escaped = encode_pdf_literal(&display);
        let _ = writeln!(
            ops,
            "/Tx BMC\nq\nBT\n{font_res} {font_size:.2} Tf\n{r:.3} {g:.3} {b:.3} rg\n2 {baseline_y:.2} Td\n{escaped} Tj\nET\nQ\nEMC"
        );
    }
    let font_res = encode_pdf_name(&widget.font_name);
    let dict_entries = format!(
        "/Type /XObject /Subtype /Form /BBox [0 0 {w:.2} {h:.2}] /Resources << /Font << {font_res} {font_id} 0 R >> >>"
    );
    let id = writer.alloc_id();
    writer.write_stream_object(id, 0, &dict_entries, ops.as_bytes());
    id
}

#[derive(Clone, Copy)]
enum CheckboxState {
    Checked,
    Unchecked,
}

fn write_checkbox_state_stream(
    writer: &mut IncrementalPdfWriter,
    widget: &WidgetAppearance,
    state: CheckboxState,
) -> u32 {
    let w = (widget.pdf_rect[2] - widget.pdf_rect[0]).abs().max(10.0);
    let h = (widget.pdf_rect[3] - widget.pdf_rect[1]).abs().max(10.0);
    let mut ops = format!(
        "1 1 1 rg 0 0 {w:.2} {h:.2} re f 0 0 0 RG 1 w 0.5 0.5 {:.2} {:.2} re S\n",
        w - 1.0,
        h - 1.0
    );
    if matches!(state, CheckboxState::Checked) {
        let _ = writeln!(
            ops,
            "0.1 0.1 0.1 RG 1.5 w {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l S",
            w * 0.2,
            h * 0.5,
            w * 0.42,
            h * 0.24,
            w * 0.8,
            h * 0.78
        );
    }
    let id = writer.alloc_id();
    let dict_entries = format!("/Type /XObject /Subtype /Form /BBox [0 0 {w:.2} {h:.2}]");
    writer.write_stream_object(id, 0, &dict_entries, ops.as_bytes());
    id
}

#[derive(Clone, Copy)]
enum RadioState {
    Selected,
    Unselected,
}

fn write_radio_state_stream(
    writer: &mut IncrementalPdfWriter,
    widget: &WidgetAppearance,
    state: RadioState,
) -> u32 {
    let w = (widget.pdf_rect[2] - widget.pdf_rect[0]).abs().max(10.0);
    let h = (widget.pdf_rect[3] - widget.pdf_rect[1]).abs().max(10.0);
    let mut ops = format!(
        "1 1 1 rg 0 0 {w:.2} {h:.2} re f 0 0 0 RG 1 w 0.5 0.5 {:.2} {:.2} re S\n",
        w - 1.0,
        h - 1.0
    );
    if matches!(state, RadioState::Selected) {
        let _ = writeln!(
            ops,
            "0.1 0.1 0.1 rg {:.2} {:.2} {:.2} {:.2} re f",
            w * 0.25,
            h * 0.25,
            w * 0.5,
            h * 0.5
        );
    }
    let id = writer.alloc_id();
    let dict_entries = format!("/Type /XObject /Subtype /Form /BBox [0 0 {w:.2} {h:.2}]");
    writer.write_stream_object(id, 0, &dict_entries, ops.as_bytes());
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::flatten::{FlattenTarget, flatten_pdf_bytes};
    use ecclesia_mupdf::{Background, Document, Matrix};

    fn build_sample_form_pdf() -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");

        let mut push_obj = |id: u32, body: &str| {
            offsets.push((id, out.len()));
            out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
        };

        push_obj(1, "<< /Type /Catalog /Pages 2 0 R /AcroForm 9 0 R >>");
        push_obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        push_obj(
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Tabs /R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R] >>",
        );
        push_obj(
            4,
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (qty) /Rect [50 700 170 724] /DA (/Helv 12 Tf 0 g) /V (2) /Ff 2 /P 3 0 R >>",
        );
        push_obj(
            5,
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (price) /Rect [50 660 170 684] /DA (/Helv 12 Tf 0 g) /V (15) /P 3 0 R >>",
        );
        push_obj(
            6,
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (total) /Rect [50 620 170 644] /DA (/Helv 12 Tf 0 g) /Ff 1 /P 3 0 R /AA << /C << /Type /Action /S /JavaScript /JS (event.value = getField\\(\"qty\"\\).value * getField\\(\"price\"\\).value;) >> /F << /Type /Action /S /JavaScript /JS (AFNumber_Format\\(2, 0, 0, 0, \"$\", true\\)) >> >> >>",
        );
        push_obj(
            7,
            "<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /Rect [50 580 70 600] /V /Off /AP << /N << /Yes 10 0 R /Off 11 0 R >> >> /P 3 0 R >>",
        );
        push_obj(
            8,
            "<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (country) /Rect [50 540 190 564] /DA (/Helv 12 Tf 0 g) /Opt [(USA) (Canada) (UK)] /V (USA) /P 3 0 R >>",
        );
        push_obj(
            9,
            "<< /Fields [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R] /CO [6 0 R] /NeedAppearances true /DR << /Font << /Helv << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>",
        );
        push_obj(
            10,
            "<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>\nstream\n\nendstream",
        );
        push_obj(
            11,
            "<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>\nstream\n\nendstream",
        );

        let xref_off = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for (_, off) in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 12 /Root 1 0 R >>\nstartxref\n{xref_off}\n%%EOF\n")
                .as_bytes(),
        );
        out
    }

    #[test]
    fn parses_fills_calculates_validates_and_round_trips_acroform() {
        let pdf = build_sample_form_pdf();
        let mut form = AcroForm::from_pdf_bytes(&pdf).expect("parse acroform");
        assert_eq!(form.fields().len(), 5);
        assert_eq!(form.calc_order(), &["total".to_owned()]);

        let calc_report = form
            .set_value("qty", FieldValue::Text("4".into()))
            .expect("set qty");
        assert_eq!(calc_report.evaluated_order, vec!["total".to_owned()]);
        assert_eq!(
            form.field("total")
                .expect("total")
                .value
                .as_display_string(),
            "$60.00"
        );

        form.set_value("agree", FieldValue::Boolean(true))
            .expect("check agree");
        form.set_value("country", FieldValue::Choice(vec!["Canada".into()]))
            .expect("choose country");

        let report = form.validate_all();
        assert!(
            report.is_valid(),
            "expected valid form, got {:?}",
            report.issues
        );

        let saved = form.to_pdf_bytes(&pdf).expect("save form");
        let reopened = AcroForm::from_pdf_bytes(&saved).expect("reopen saved form");
        assert_eq!(
            reopened
                .field("qty")
                .expect("qty")
                .value
                .as_display_string(),
            "4"
        );
        assert_eq!(
            reopened
                .field("total")
                .expect("total")
                .value
                .as_display_string(),
            "$60.00"
        );
        assert_eq!(
            reopened.field("agree").expect("agree").value,
            FieldValue::Boolean(true)
        );
        assert_eq!(
            reopened.field("country").expect("country").value,
            FieldValue::Choice(vec!["Canada".to_owned()])
        );

        // Verify MuPDF renders the saved PDF cleanly.
        let doc = Document::from_bytes(&saved, "application/pdf").expect("open in mupdf");
        assert_eq!(doc.page_count().expect("pages"), 1);
    }

    #[test]
    fn calc_dag_supports_af_simple_calculate_and_detects_cycles() {
        let mut form = AcroForm::new();
        form.add_field(FormField::text("a", "10"));
        form.add_field(FormField::text("b", "20"));
        let mut sum = FormField::text("sum", "");
        sum.calculate_script = Some(r#"AFSimple_Calculate("SUM", new Array("a", "b"))"#.to_owned());
        form.add_field(sum);

        let mut avg = FormField::text("avg", "");
        avg.calculate_script = Some(r#"AFSimple_Calculate("AVG", ["a", "sum"])"#.to_owned());
        form.add_field(avg);

        let rep = form.recalculate().expect("recalculate");
        assert_eq!(rep.evaluated_order, vec!["sum", "avg"]);
        assert_eq!(
            form.field("sum").expect("sum").value.as_display_string(),
            "30"
        );
        assert_eq!(
            form.field("avg").expect("avg").value.as_display_string(),
            "20"
        );

        // Cycle check.
        form.field_mut("a").expect("a").calculate_script =
            Some(r#"event.value = getField("b").value + 1"#.to_owned());
        form.field_mut("b").expect("b").calculate_script =
            Some(r#"event.value = getField("a").value + 1"#.to_owned());
        assert!(matches!(
            form.recalculate(),
            Err(FormError::Calculation(CalcError::CyclicDependency(_)))
        ));
    }

    #[test]
    fn validation_checks_required_range_length_email_date_and_regex() {
        let mut form = AcroForm::new();
        let mut email = FormField::text("email", "bad-email");
        email.flags = email.flags.with(FieldFlags::REQUIRED);
        email.validation.push(ValidationRule::Email);
        form.add_field(email);

        let mut age = FormField::text("age", "150");
        age.validation.push(ValidationRule::NumericRange {
            min: Some(0.0),
            max: Some(120.0),
        });
        form.add_field(age);

        let mut zip = FormField::text("zip", "ABC");
        zip.validation
            .push(ValidationRule::Pattern(r"^\d{5}(-\d{4})?$".to_owned()));
        form.add_field(zip);

        let mut date = FormField::text("dob", "2026-02-30");
        date.validation
            .push(ValidationRule::DateFormat("YYYY-MM-DD".to_owned()));
        form.add_field(date);

        let rep = form.validate_all();
        assert_eq!(rep.issues.len(), 4, "issues: {:?}", rep.issues);

        form.set_value("email", FieldValue::Text("ada@example.org".into()))
            .expect("set email");
        form.set_value("age", FieldValue::Text("42".into()))
            .expect("set age");
        form.set_value("zip", FieldValue::Text("10001-1234".into()))
            .expect("set zip");
        form.set_value("dob", FieldValue::Text("2024-02-29".into()))
            .expect("set leap date");
        assert!(form.validate_all().is_valid());
    }

    #[test]
    fn tab_order_respects_page_and_top_left_geometry() {
        let mut form = AcroForm::new();
        let mut f_bottom = FormField::text("bottom", "");
        f_bottom.widgets.push(WidgetAppearance {
            page: 0,
            rect: PageRect {
                x0: 50.0,
                y0: 200.0,
                x1: 150.0,
                y1: 220.0,
            },
            ..Default::default()
        });
        let mut f_top_right = FormField::text("top_right", "");
        f_top_right.widgets.push(WidgetAppearance {
            page: 0,
            rect: PageRect {
                x0: 200.0,
                y0: 50.0,
                x1: 300.0,
                y1: 70.0,
            },
            ..Default::default()
        });
        let mut f_top_left = FormField::text("top_left", "");
        f_top_left.widgets.push(WidgetAppearance {
            page: 0,
            rect: PageRect {
                x0: 50.0,
                y0: 50.0,
                x1: 150.0,
                y1: 70.0,
            },
            ..Default::default()
        });
        form.add_field(f_bottom);
        form.add_field(f_top_right);
        form.add_field(f_top_left);

        let names: Vec<&str> = form
            .tab_ordered_fields()
            .into_iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["top_left", "top_right", "bottom"]);
    }

    #[test]
    fn flattened_forms_keep_visual_pixels_and_remove_acroform_widgets() {
        let pdf = build_sample_form_pdf();
        let mut form = AcroForm::from_pdf_bytes(&pdf).expect("parse");
        form.set_value("qty", FieldValue::Text("7".into()))
            .expect("qty");
        form.set_value("agree", FieldValue::Boolean(true))
            .expect("agree");
        let filled = form.to_pdf_bytes(&pdf).expect("save filled");

        let flattened =
            flatten_pdf_bytes(&filled, FlattenTarget::FormsAndAnnotations).expect("flatten");
        let after_form = AcroForm::from_pdf_bytes(&flattened).expect("parse flattened");
        assert!(
            after_form.fields().is_empty(),
            "flattened copy must have no fields"
        );

        let before_doc = Document::from_bytes(&filled, "application/pdf").expect("open filled");
        let after_doc =
            Document::from_bytes(&flattened, "application/pdf").expect("open flattened");
        let before_pix = before_doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render filled");
        let after_pix = after_doc
            .load_page(0)
            .expect("p0")
            .render(&Matrix::IDENTITY, Background::White)
            .expect("render flattened");

        assert_eq!(before_pix.width(), after_pix.width());
        assert_eq!(before_pix.height(), after_pix.height());
        assert_eq!(
            before_pix.samples(),
            after_pix.samples(),
            "flattened page pixels must match filled form pixels"
        );
    }
}
