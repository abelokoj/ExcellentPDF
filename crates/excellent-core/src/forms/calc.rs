//! PDF calculation DAG and safe AcroForm JavaScript-subset evaluator (plan O.11).
//!
//! Supports `AFSimple_Calculate` (`SUM`, `AVG`, `MIN`, `MAX`, `PRD`), `AFNumber_Format`,
//! `AFPercent_Format`, `AFDate_FormatEx`, `AFRange_Validate`, and safe arithmetic expressions
//! referencing `getField("...").value`. Execution is strictly bounded with cycle detection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Maximum AST evaluation depth to prevent stack exhaustion on untrusted scripts.
const MAX_EVAL_DEPTH: usize = 64;

/// Maximum tokens accepted in a single AcroForm expression.
const MAX_TOKENS: usize = 1024;

/// Aggregation operation supported by `AFSimple_Calculate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimpleCalcOp {
    /// Sum of referenced numeric values (`SUM`).
    Sum,
    /// Arithmetic mean of non-empty referenced numeric values (`AVG`).
    Avg,
    /// Minimum of referenced numeric values (`MIN`).
    Min,
    /// Maximum of referenced numeric values (`MAX`).
    Max,
    /// Product of referenced numeric values (`PRD`).
    Prd,
}

impl SimpleCalcOp {
    /// Parses a case-insensitive `AFSimple_Calculate` operation name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "SUM" => Some(Self::Sum),
            "AVG" => Some(Self::Avg),
            "MIN" => Some(Self::Min),
            "MAX" => Some(Self::Max),
            "PRD" => Some(Self::Prd),
            _ => None,
        }
    }

    /// Canonical PDF JavaScript operator string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::Prd => "PRD",
        }
    }
}

/// Thousands/decimal separator style used by `AFNumber_Format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NumberSepStyle {
    /// `1,234.56` (comma thousands, dot decimal).
    #[default]
    CommaDot,
    /// `1234.56` (no thousands, dot decimal).
    NoneDot,
    /// `1.234,56` (dot thousands, comma decimal).
    DotComma,
    /// `1234,56` (no thousands, comma decimal).
    NoneComma,
}

impl NumberSepStyle {
    /// Converts Acrobat's `sepStyle` integer (`0..=3`) to a [`NumberSepStyle`].
    #[must_use]
    pub const fn from_code(code: i32) -> Self {
        match code {
            1 => Self::NoneDot,
            2 => Self::DotComma,
            3 => Self::NoneComma,
            _ => Self::CommaDot,
        }
    }
}

/// Negative number display style used by `AFNumber_Format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NegativeStyle {
    /// Leading minus sign (`-12.34`).
    #[default]
    Minus,
    /// Parentheses (`(12.34)`).
    Parens,
}

/// Placement of currency symbol relative to the formatted number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CurrencyPlacement {
    /// Prefix (`$12.34`).
    #[default]
    Prepend,
    /// Suffix (`12.34$`).
    Append,
}

/// Parsed `AFNumber_Format` specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberFormatSpec {
    /// Number of fractional digits (`nDec`).
    pub decimals: usize,
    /// Separator style (`sepStyle`).
    pub sep_style: NumberSepStyle,
    /// Negative formatting (`negStyle`).
    pub neg_style: NegativeStyle,
    /// Currency symbol string (`strCurrency`), empty for none.
    pub currency: String,
    /// Whether currency is prepended or appended (`bCurrencyPrepend`).
    pub currency_placement: CurrencyPlacement,
}

impl Default for NumberFormatSpec {
    fn default() -> Self {
        Self {
            decimals: 2,
            sep_style: NumberSepStyle::CommaDot,
            neg_style: NegativeStyle::Minus,
            currency: String::new(),
            currency_placement: CurrencyPlacement::Prepend,
        }
    }
}

/// Parsed AcroForm calculation action.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CalcAction {
    /// `AFSimple_Calculate(op, fields)`.
    Simple {
        /// Aggregation operator.
        op: SimpleCalcOp,
        /// Referenced field names.
        fields: Vec<String>,
    },
    /// Safe arithmetic expression reading `getField("name").value` or `event.value = ...`.
    Expression {
        /// Normalized expression source.
        source: String,
        /// Referenced field names extracted statically for DAG ordering.
        dependencies: Vec<String>,
    },
}

/// Error produced when parsing or running form calculation scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum CalcError {
    /// A cyclic dependency was detected between calculated fields.
    CyclicDependency(Vec<String>),
    /// The script uses syntax outside the safe AcroForm subset.
    UnsupportedScript(String),
    /// Evaluation exceeded depth or token bounds.
    BudgetExceeded(String),
    /// Division by zero or invalid numeric operation.
    EvalFailure(String),
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CyclicDependency(cycle) => {
                write!(
                    f,
                    "cyclic form calculation dependency: {}",
                    cycle.join(" -> ")
                )
            }
            Self::UnsupportedScript(msg) => write!(f, "unsupported form script: {msg}"),
            Self::BudgetExceeded(msg) => write!(f, "form script budget exceeded: {msg}"),
            Self::EvalFailure(msg) => write!(f, "form script evaluation failed: {msg}"),
        }
    }
}

impl std::error::Error for CalcError {}

/// Outcome of running the calculation DAG over a form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CalculationReport {
    /// Fields evaluated in topological order.
    pub evaluated_order: Vec<String>,
    /// Fields whose value changed during recalculation.
    pub updated_fields: Vec<String>,
}

/// Parses an AcroForm `/AA /C` calculation script into a [`CalcAction`].
///
/// # Errors
///
/// Returns [`CalcError::UnsupportedScript`] if the script cannot be parsed safely.
pub fn parse_calc_script(script: &str) -> Result<CalcAction, CalcError> {
    let trimmed = script.trim().trim_end_matches(';').trim();
    if let Some(simple) = try_parse_af_simple_calculate(trimmed)? {
        return Ok(simple);
    }
    let expr_src = if let Some(rhs) = strip_event_value_assignment(trimmed) {
        rhs.trim()
    } else {
        trimmed
    };
    if expr_src.is_empty() {
        return Err(CalcError::UnsupportedScript(
            "empty calculation script".to_owned(),
        ));
    }
    let tokens = tokenize_expr(expr_src)?;
    let mut deps = BTreeSet::new();
    for tok in &tokens {
        if let Token::GetField(name) = tok {
            deps.insert(name.clone());
        }
    }
    // Validate syntax once up front.
    let mut parser = ExprParser {
        tokens: &tokens,
        pos: 0,
    };
    let _ast = parser.parse_expr(0)?;
    if parser.pos != tokens.len() {
        return Err(CalcError::UnsupportedScript(format!(
            "unexpected trailing tokens in expression: {expr_src}"
        )));
    }
    Ok(CalcAction::Expression {
        source: expr_src.to_owned(),
        dependencies: deps.into_iter().collect(),
    })
}

/// Parses `AFNumber_Format(nDec, sepStyle, negStyle, currStyle, strCurrency, bCurrencyPrepend)`.
#[must_use]
pub fn parse_af_number_format(script: &str) -> Option<NumberFormatSpec> {
    let args = extract_call_args(script, "AFNumber_Format")?;
    let decimals = args.first()?.trim().parse::<usize>().ok()?.min(12);
    let sep_style = args
        .get(1)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map_or(NumberSepStyle::CommaDot, NumberSepStyle::from_code);
    let neg_style = args
        .get(2)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map_or(NegativeStyle::Minus, |code| {
            if code == 2 || code == 3 {
                NegativeStyle::Parens
            } else {
                NegativeStyle::Minus
            }
        });
    let currency = args
        .get(4)
        .map(|s| unquote_js_string(s.trim()))
        .unwrap_or_default();
    let currency_placement = args
        .get(5)
        .map(|s| s.trim().eq_ignore_ascii_case("false"))
        .map_or(CurrencyPlacement::Prepend, |is_false| {
            if is_false {
                CurrencyPlacement::Append
            } else {
                CurrencyPlacement::Prepend
            }
        });
    Some(NumberFormatSpec {
        decimals,
        sep_style,
        neg_style,
        currency,
        currency_placement,
    })
}

/// Parses `AFPercent_Format(nDec, sepStyle)` into decimals and separator style.
#[must_use]
pub fn parse_af_percent_format(script: &str) -> Option<(usize, NumberSepStyle)> {
    let args = extract_call_args(script, "AFPercent_Format")?;
    let decimals = args.first()?.trim().parse::<usize>().ok()?.min(12);
    let sep_style = args
        .get(1)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map_or(NumberSepStyle::CommaDot, NumberSepStyle::from_code);
    Some((decimals, sep_style))
}

/// Parses `AFDate_FormatEx("yyyy-mm-dd")` and returns the date format string.
#[must_use]
pub fn parse_af_date_format(script: &str) -> Option<String> {
    let args = extract_call_args(script, "AFDate_FormatEx")?;
    let fmt = unquote_js_string(args.first()?.trim());
    (!fmt.is_empty()).then_some(fmt)
}

/// Parses `AFRange_Validate(bGreaterThan, nGreaterThan, bLessThan, nLessThan)` into `(min, max)`.
#[must_use]
pub fn parse_af_range_validate(script: &str) -> Option<(Option<f64>, Option<f64>)> {
    let args = extract_call_args(script, "AFRange_Validate")?;
    if args.len() < 4 {
        return None;
    }
    let has_min = args[0].trim().eq_ignore_ascii_case("true");
    let min_val = if has_min {
        args[1].trim().parse::<f64>().ok()
    } else {
        None
    };
    let has_max = args[2].trim().eq_ignore_ascii_case("true");
    let max_val = if has_max {
        args[3].trim().parse::<f64>().ok()
    } else {
        None
    };
    Some((min_val, max_val))
}

/// Formats `value` according to `spec` (matching Acrobat `AFNumber_Format`).
#[must_use]
pub fn format_number(value: f64, spec: &NumberFormatSpec) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let is_neg = value < 0.0;
    let abs = value.abs();
    let fixed = format!("{abs:.prec$}", prec = spec.decimals);
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (fixed.as_str(), None),
    };

    let (thousands_sep, decimal_sep) = match spec.sep_style {
        NumberSepStyle::CommaDot => (Some(','), '.'),
        NumberSepStyle::NoneDot => (None, '.'),
        NumberSepStyle::DotComma => (Some('.'), ','),
        NumberSepStyle::NoneComma => (None, ','),
    };

    let grouped_int = if let Some(sep) = thousands_sep {
        group_thousands(int_part, sep)
    } else {
        int_part.to_owned()
    };

    let mut num_str = grouped_int;
    if let Some(frac) = frac_part
        && spec.decimals > 0
    {
        num_str.push(decimal_sep);
        num_str.push_str(frac);
    }

    let with_currency = if spec.currency.is_empty() {
        num_str
    } else {
        match spec.currency_placement {
            CurrencyPlacement::Prepend => format!("{}{num_str}", spec.currency),
            CurrencyPlacement::Append => format!("{num_str}{}", spec.currency),
        }
    };

    if is_neg {
        match spec.neg_style {
            NegativeStyle::Minus => format!("-{with_currency}"),
            NegativeStyle::Parens => format!("({with_currency})"),
        }
    } else {
        with_currency
    }
}

/// Parses a numeric field string, stripping thousands separators, currency signs, and parenthesized negatives.
#[must_use]
pub fn parse_lenient_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, inner) =
        if let Some(s) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            (true, s.trim())
        } else {
            (false, trimmed)
        };
    let cleaned: String = inner
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let parsed = cleaned.parse::<f64>().ok()?;
    if negative {
        Some(-parsed.abs())
    } else {
        Some(parsed)
    }
}

/// Evaluates `action` given the current map of field name -> string value.
///
/// # Errors
///
/// Returns [`CalcError`] if parsing or arithmetic evaluation fails.
pub fn evaluate_calc_action(
    action: &CalcAction,
    values: &BTreeMap<String, String>,
) -> Result<f64, CalcError> {
    match action {
        CalcAction::Simple { op, fields } => {
            let nums: Vec<f64> = fields
                .iter()
                .filter_map(|name| values.get(name).and_then(|v| parse_lenient_number(v)))
                .collect();
            let result = match op {
                SimpleCalcOp::Sum => nums.iter().copied().sum(),
                SimpleCalcOp::Avg => {
                    if nums.is_empty() {
                        0.0
                    } else {
                        nums.iter().copied().sum::<f64>() / (nums.len() as f64)
                    }
                }
                SimpleCalcOp::Min => nums.iter().copied().reduce(f64::min).unwrap_or(0.0),
                SimpleCalcOp::Max => nums.iter().copied().reduce(f64::max).unwrap_or(0.0),
                SimpleCalcOp::Prd => {
                    if nums.is_empty() {
                        0.0
                    } else {
                        nums.iter().copied().product()
                    }
                }
            };
            Ok(result)
        }
        CalcAction::Expression { source, .. } => {
            let tokens = tokenize_expr(source)?;
            let mut parser = ExprParser {
                tokens: &tokens,
                pos: 0,
            };
            let ast = parser.parse_expr(0)?;
            eval_ast(&ast, values, 0)
        }
    }
}

/// Topologically orders calculated fields by their dependencies, honoring `explicit_order` (`/CO`) as a tie-breaker.
///
/// # Errors
///
/// Returns [`CalcError::CyclicDependency`] if a cycle exists among calculated fields.
pub fn topological_calc_order(
    actions: &BTreeMap<String, CalcAction>,
    explicit_order: &[String],
) -> Result<Vec<String>, CalcError> {
    let mut deps_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut in_degree: BTreeMap<&str, usize> = BTreeMap::new();

    for name in actions.keys() {
        in_degree.insert(name.as_str(), 0);
        deps_of.insert(name.as_str(), Vec::new());
    }

    for (target, action) in actions {
        let refs = match action {
            CalcAction::Simple { fields, .. } => fields.as_slice(),
            CalcAction::Expression { dependencies, .. } => dependencies.as_slice(),
        };
        let mut seen = BTreeSet::new();
        for dep in refs {
            if dep == target {
                return Err(CalcError::CyclicDependency(vec![
                    target.clone(),
                    dep.clone(),
                ]));
            }
            if actions.contains_key(dep) && seen.insert(dep.as_str()) {
                deps_of
                    .get_mut(target.as_str())
                    .expect("target exists")
                    .push(dep.as_str());
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(target.as_str());
                *in_degree.get_mut(target.as_str()).expect("target exists") += 1;
            }
        }
    }

    let rank = |name: &str| -> usize {
        explicit_order
            .iter()
            .position(|item| item == name)
            .unwrap_or(usize::MAX)
    };

    let mut ready: Vec<&str> = in_degree
        .iter()
        .filter_map(|(&k, &deg)| (deg == 0).then_some(k))
        .collect();
    ready.sort_by_key(|&name| (rank(name), name));
    let mut queue: VecDeque<&str> = ready.into_iter().collect();

    let mut order = Vec::with_capacity(actions.len());
    while let Some(node) = queue.pop_front() {
        order.push(node.to_owned());
        if let Some(children) = dependents.get(node) {
            let mut newly_ready = Vec::new();
            for &child in children {
                let deg = in_degree.get_mut(child).expect("child exists");
                *deg -= 1;
                if *deg == 0 {
                    newly_ready.push(child);
                }
            }
            newly_ready.sort_by_key(|&name| (rank(name), name));
            for item in newly_ready {
                queue.push_back(item);
            }
        }
    }

    if order.len() != actions.len() {
        let cycle: Vec<String> = in_degree
            .into_iter()
            .filter(|&(_, deg)| deg > 0)
            .map(|(k, _)| k.to_owned())
            .collect();
        return Err(CalcError::CyclicDependency(cycle));
    }
    Ok(order)
}

fn group_thousands(int_part: &str, sep: char) -> String {
    let len = int_part.len();
    if len <= 3 {
        return int_part.to_owned();
    }
    let mut out = String::with_capacity(len + len / 3);
    for (idx, ch) in int_part.chars().enumerate() {
        if idx > 0 && (len - idx).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

fn extract_call_args(script: &str, fn_name: &str) -> Option<Vec<String>> {
    let pos = script.find(fn_name)?;
    let after = script[pos + fn_name.len()..].trim_start();
    let after_paren = after.strip_prefix('(')?;
    let mut depth = 1usize;
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    let mut end = None;
    for (idx, ch) in after_paren.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_quote.is_some() {
            if ch == '\\' {
                escaped = true;
            } else if Some(ch) == in_quote {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &after_paren[..end?];
    Some(split_top_level_args(inner))
}

fn split_top_level_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escaped = false;

    for ch in s.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if in_quote.is_some() {
            cur.push(ch);
            if ch == '\\' {
                escaped = true;
            } else if Some(ch) == in_quote {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_quote = Some(ch);
                cur.push(ch);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                args.push(cur.trim().to_owned());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_owned());
    }
    args
}

fn unquote_js_string(s: &str) -> String {
    let trimmed = s.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|r| r.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    inner
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\")
}

fn try_parse_af_simple_calculate(script: &str) -> Result<Option<CalcAction>, CalcError> {
    let Some(args) = extract_call_args(script, "AFSimple_Calculate") else {
        return Ok(None);
    };
    if args.len() < 2 {
        return Err(CalcError::UnsupportedScript(
            "AFSimple_Calculate requires operator and field list".to_owned(),
        ));
    }
    let op_name = unquote_js_string(&args[0]);
    let op = SimpleCalcOp::from_name(&op_name).ok_or_else(|| {
        CalcError::UnsupportedScript(format!("unknown AFSimple_Calculate op {op_name:?}"))
    })?;
    let list_arg = args[1].trim();
    let inner = if let Some(rest) = list_arg.strip_prefix("new Array") {
        rest.trim()
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
            .unwrap_or("")
    } else if let Some(rest) = list_arg.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        rest
    } else {
        list_arg
    };
    let fields: Vec<String> = split_top_level_args(inner)
        .into_iter()
        .map(|s| unquote_js_string(&s))
        .filter(|s| !s.is_empty())
        .collect();
    Ok(Some(CalcAction::Simple { op, fields }))
}

fn strip_event_value_assignment(script: &str) -> Option<&str> {
    let rest = script.strip_prefix("event")?;
    let rest = rest.trim_start().strip_prefix('.')?;
    let rest = rest.trim_start().strip_prefix("value")?;
    let rest = rest.trim_start().strip_prefix('=')?;
    Some(rest)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    GetField(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
}

fn tokenize_expr(src: &str) -> Result<Vec<Token>, CalcError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if tokens.len() >= MAX_TOKENS {
            return Err(CalcError::BudgetExceeded(
                "expression token count limit reached".to_owned(),
            ));
        }
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        match ch {
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                let num = raw.parse::<f64>().map_err(|_| {
                    CalcError::UnsupportedScript(format!("invalid numeric literal {raw:?}"))
                })?;
                tokens.push(Token::Num(num));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
                {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                if ident == "this.getField" || ident == "getField" {
                    // Consume `("fieldName").value`
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }
                    if chars.get(i) != Some(&'(') {
                        return Err(CalcError::UnsupportedScript(
                            "expected '(' after getField".to_owned(),
                        ));
                    }
                    i += 1;
                    let arg_start = i;
                    while i < chars.len() && chars[i] != ')' {
                        i += 1;
                    }
                    if chars.get(i) != Some(&')') {
                        return Err(CalcError::UnsupportedScript(
                            "unterminated getField(...)".to_owned(),
                        ));
                    }
                    let raw_name: String = chars[arg_start..i].iter().collect();
                    i += 1; // skip ')'
                    // Skip optional `.value` or `.valueAsString`
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }
                    if chars.get(i) == Some(&'.') {
                        i += 1;
                        while i < chars.len()
                            && (chars[i].is_ascii_alphanumeric() || chars[i] == '_')
                        {
                            i += 1;
                        }
                    }
                    tokens.push(Token::GetField(unquote_js_string(&raw_name)));
                } else {
                    tokens.push(Token::Ident(ident));
                }
            }
            other => {
                return Err(CalcError::UnsupportedScript(format!(
                    "unexpected character {other:?} in expression"
                )));
            }
        }
    }
    Ok(tokens)
}

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Field(String),
    Neg(Box<Self>),
    Binary(BinOp, Box<Self>, Box<Self>),
    Call(String, Vec<Self>),
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl ExprParser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn parse_expr(&mut self, depth: usize) -> Result<Expr, CalcError> {
        if depth > MAX_EVAL_DEPTH {
            return Err(CalcError::BudgetExceeded(
                "expression nesting exceeds depth limit".to_owned(),
            ));
        }
        let mut lhs = self.parse_term(depth + 1)?;
        while let Some(tok) = self.peek() {
            let op = match tok {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_term(depth + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_term(&mut self, depth: usize) -> Result<Expr, CalcError> {
        let mut lhs = self.parse_unary(depth + 1)?;
        while let Some(tok) = self.peek() {
            let op = match tok {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary(depth + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self, depth: usize) -> Result<Expr, CalcError> {
        if depth > MAX_EVAL_DEPTH {
            return Err(CalcError::BudgetExceeded(
                "expression nesting exceeds depth limit".to_owned(),
            ));
        }
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.parse_unary(depth + 1)?)))
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.parse_unary(depth + 1)
            }
            _ => self.parse_primary(depth + 1),
        }
    }

    fn parse_primary(&mut self, depth: usize) -> Result<Expr, CalcError> {
        match self.peek().cloned() {
            Some(Token::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Token::GetField(name)) => {
                self.pos += 1;
                Ok(Expr::Field(name))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let expr = self.parse_expr(depth + 1)?;
                if self.peek() != Some(&Token::RParen) {
                    return Err(CalcError::UnsupportedScript("missing ')'".to_owned()));
                }
                self.pos += 1;
                Ok(expr)
            }
            Some(Token::Ident(name)) => {
                self.pos += 1;
                if self.peek() == Some(&Token::LParen) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Token::RParen) {
                        loop {
                            args.push(self.parse_expr(depth + 1)?);
                            if self.peek() == Some(&Token::Comma) {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    if self.peek() != Some(&Token::RParen) {
                        return Err(CalcError::UnsupportedScript(
                            "missing ')' after function call".to_owned(),
                        ));
                    }
                    self.pos += 1;
                    Ok(Expr::Call(name, args))
                } else {
                    Err(CalcError::UnsupportedScript(format!(
                        "unsupported identifier {name:?}"
                    )))
                }
            }
            other => Err(CalcError::UnsupportedScript(format!(
                "unexpected token {other:?}"
            ))),
        }
    }
}

fn eval_ast(
    expr: &Expr,
    values: &BTreeMap<String, String>,
    depth: usize,
) -> Result<f64, CalcError> {
    if depth > MAX_EVAL_DEPTH {
        return Err(CalcError::BudgetExceeded(
            "evaluation recursion limit reached".to_owned(),
        ));
    }
    match expr {
        Expr::Num(n) => Ok(*n),
        Expr::Field(name) => Ok(values
            .get(name)
            .and_then(|v| parse_lenient_number(v))
            .unwrap_or(0.0)),
        Expr::Neg(inner) => Ok(-eval_ast(inner, values, depth + 1)?),
        Expr::Binary(op, lhs, rhs) => {
            let l = eval_ast(lhs, values, depth + 1)?;
            let r = eval_ast(rhs, values, depth + 1)?;
            match op {
                BinOp::Add => Ok(l + r),
                BinOp::Sub => Ok(l - r),
                BinOp::Mul => Ok(l * r),
                BinOp::Div => {
                    if r.abs() < f64::EPSILON {
                        Ok(0.0)
                    } else {
                        Ok(l / r)
                    }
                }
            }
        }
        Expr::Call(name, args) => {
            let eval_args: Vec<f64> = args
                .iter()
                .map(|a| eval_ast(a, values, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            match name.as_str() {
                "Number" => Ok(eval_args.first().copied().unwrap_or(0.0)),
                "Math.round" => Ok(eval_args.first().copied().unwrap_or(0.0).round()),
                "Math.floor" => Ok(eval_args.first().copied().unwrap_or(0.0).floor()),
                "Math.ceil" => Ok(eval_args.first().copied().unwrap_or(0.0).ceil()),
                "Math.abs" => Ok(eval_args.first().copied().unwrap_or(0.0).abs()),
                "Math.min" => Ok(eval_args.into_iter().reduce(f64::min).unwrap_or(0.0)),
                "Math.max" => Ok(eval_args.into_iter().reduce(f64::max).unwrap_or(0.0)),
                other => Err(CalcError::UnsupportedScript(format!(
                    "disallowed function call {other:?}"
                ))),
            }
        }
    }
}
