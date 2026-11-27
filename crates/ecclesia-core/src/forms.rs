//! PDF interactive forms engine: field model, validation, calculation DAG, and flattening
//! (plan O.11, O.13).

mod calc;
mod flatten;
mod model;
pub(crate) mod pdf_codec;

pub use calc::{
    CalcAction, CalcError, CalculationReport, CurrencyPlacement, NegativeStyle, NumberFormatSpec,
    NumberSepStyle, SimpleCalcOp, evaluate_calc_action, format_number, parse_af_date_format,
    parse_af_number_format, parse_af_percent_format, parse_af_range_validate, parse_calc_script,
    parse_lenient_number, topological_calc_order,
};
pub use flatten::{FlattenTarget, flatten_pdf_bytes, save_flattened_forms};
pub use model::{
    AcroForm, ChoiceKind, ChoiceOption, FieldFlags, FieldKind, FieldValidation, FieldValue,
    FormError, FormField, RadioOption, ValidationIssue, ValidationReport, ValidationRule,
    WidgetAppearance, matches_regex,
};
