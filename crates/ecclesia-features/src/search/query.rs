//! Rich search query parser, FTS5 translator, and saved-search smart folders (plan F.4, F.5).
//!
//! Parses boolean expressions (`AND`, `OR`, `NOT`, `-term`, parentheses), quoted phrases (`"..."`),
//! wildcards (`term*`, `*sub*`), fuzzy terms (`term~`, `term~2`), regexes (`/pattern/`), and field
//! operators (`author:`, `tag:`, `color:`, `after:`, `before:`, `page:`, `kind:` / `in:`).

use std::fmt;
use std::path::PathBuf;

use ecclesia_store::fts::{FtsHit, SavedSearch, SearchFilters, SourceKind, escape_fts5_term};
use rusqlite::{Connection, Transaction};

/// Error returned when a search query is malformed (plan F.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParseError {
    /// Character offset in the input query where the problem occurred.
    pub position: usize,
    /// Helpful human-readable diagnostic message.
    pub message: String,
}

impl fmt::Display for QueryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "query error at position {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for QueryParseError {}

/// Page comparison filter parsed from `page:...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFilter {
    /// Exact 1-based page (`page:5`).
    Exact(usize),
    /// Strictly greater than (`page:>100`).
    GreaterThan(usize),
    /// Greater than or equal (`page:>=100`).
    GreaterOrEqual(usize),
    /// Strictly less than (`page:<10`).
    LessThan(usize),
    /// Less than or equal (`page:<=10`).
    LessOrEqual(usize),
    /// Inclusive 1-based range (`page:5..12`).
    Range(usize, usize),
}

impl PageFilter {
    /// Converts a 1-based user page constraint into inclusive 0-based `(page_min, page_max)`.
    #[must_use]
    pub fn to_zero_based_bounds(self) -> (Option<usize>, Option<usize>) {
        match self {
            Self::Exact(p) => {
                let z = p.saturating_sub(1);
                (Some(z), Some(z))
            }
            Self::GreaterThan(p) => (Some(p), None),
            Self::GreaterOrEqual(p) => (Some(p.saturating_sub(1)), None),
            Self::LessThan(p) => (None, Some(p.saturating_sub(2))),
            Self::LessOrEqual(p) => (None, Some(p.saturating_sub(1))),
            Self::Range(lo, hi) => (Some(lo.saturating_sub(1)), Some(hi.saturating_sub(1))),
        }
    }
}

/// Abstract syntax tree of a parsed search query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryExpr {
    /// Empty query matching everything allowed by filters.
    All,
    /// Bare word token.
    Term(String),
    /// Exact quoted phrase.
    Phrase(String),
    /// Prefix wildcard (`foo*`).
    Prefix(String),
    /// General wildcard containing `*` or `?` (`*bar`, `f*o`).
    Wildcard(String),
    /// Fuzzy term (`foo~` or `foo~2`) with maximum edit distance.
    Fuzzy {
        /// Base word.
        term: String,
        /// Maximum Levenshtein distance (default `1`, capped at `3`).
        max_edits: usize,
    },
    /// Regular expression (`/pattern/`) evaluated on candidate rows.
    Regex(RegexPattern),
    /// Logical conjunction.
    And(Box<Self>, Box<Self>),
    /// Logical disjunction.
    Or(Box<Self>, Box<Self>),
    /// Logical negation.
    Not(Box<Self>),
}

/// Parsed query separated into SQL/FTS5 parts and in-memory post-predicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Raw query string as entered by the user.
    pub raw: String,
    /// Boolean expression over text/regex/fuzzy terms.
    pub expr: QueryExpr,
    /// Structured SQL filters (`author:`, `tag:`, `color:`, `after:`, `before:`, `page:`, `kind:`).
    pub filters: SearchFilters,
    /// Optional FTS5 `MATCH` expression when translatable directly to SQLite FTS5.
    pub fts_match: Option<String>,
    /// Optional substring query for trigram lookup (`*sub*`).
    pub trigram_hint: Option<String>,
    /// Whether post-filtering (fuzzy, general wildcard, or regex) is required on candidate hits.
    pub needs_post_filter: bool,
}

impl ParsedQuery {
    /// Parses `input` into a [`ParsedQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`QueryParseError`] with position and explanation when operators, quotes,
    /// parentheses, dates, page numbers, or regexes are malformed.
    pub fn parse(input: &str) -> Result<Self, QueryParseError> {
        let tokens = tokenize(input)?;
        let mut filters = SearchFilters::default();
        let mut expr_tokens = Vec::new();

        for tok in tokens {
            match tok.kind {
                TokenKind::Field { key, value } => {
                    apply_field_filter(&mut filters, &key, &value, tok.pos)?;
                }
                _ => expr_tokens.push(tok),
            }
        }

        let expr = if expr_tokens.is_empty() {
            QueryExpr::All
        } else {
            let mut parser = Parser {
                tokens: &expr_tokens,
                pos: 0,
            };
            let parsed = parser.parse_or()?;
            if let Some(leftover) = parser.peek() {
                return Err(QueryParseError {
                    position: leftover.pos,
                    message: format!("unexpected token `{}`", leftover.raw),
                });
            }
            parsed
        };

        let fts_match = build_fts5_expr(&expr);
        let trigram_hint = find_trigram_hint(&expr);
        let needs_post_filter = expr_needs_post_filter(&expr);

        Ok(Self {
            raw: input.to_owned(),
            expr,
            filters,
            fts_match,
            trigram_hint,
            needs_post_filter,
        })
    }

    /// Evaluates any regex, fuzzy, or wildcard predicates against `hit`.
    #[must_use]
    pub fn matches_hit(&self, hit: &FtsHit) -> bool {
        eval_expr_against_text(&self.expr, &hit.title, &hit.body)
    }
}

/// Evaluates whether a boolean [`QueryExpr`] matches `(title, body)` in memory.
#[must_use]
pub fn eval_expr_against_text(expr: &QueryExpr, title: &str, body: &str) -> bool {
    match expr {
        QueryExpr::All => true,
        QueryExpr::Term(t) => {
            let needle = t.to_lowercase();
            title.to_lowercase().contains(&needle) || body.to_lowercase().contains(&needle)
        }
        QueryExpr::Phrase(p) => {
            let needle = p.to_lowercase();
            title.to_lowercase().contains(&needle) || body.to_lowercase().contains(&needle)
        }
        QueryExpr::Prefix(p) => {
            let needle = p.to_lowercase();
            words_of(title)
                .chain(words_of(body))
                .any(|w| w.to_lowercase().starts_with(&needle))
        }
        QueryExpr::Wildcard(pat) => {
            let pat_lower = pat.to_lowercase();
            words_of(title)
                .chain(words_of(body))
                .any(|w| glob_match(&pat_lower, &w.to_lowercase()))
        }
        QueryExpr::Fuzzy { term, max_edits } => {
            let target = term.to_lowercase();
            words_of(title)
                .chain(words_of(body))
                .any(|w| levenshtein_within(&target, &w.to_lowercase(), *max_edits))
        }
        QueryExpr::Regex(re) => re.is_match(title) || re.is_match(body),
        QueryExpr::And(a, b) => {
            eval_expr_against_text(a, title, body) && eval_expr_against_text(b, title, body)
        }
        QueryExpr::Or(a, b) => {
            eval_expr_against_text(a, title, body) || eval_expr_against_text(b, title, body)
        }
        QueryExpr::Not(inner) => !eval_expr_against_text(inner, title, body),
    }
}

fn words_of(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| !w.is_empty())
}

fn expr_needs_post_filter(expr: &QueryExpr) -> bool {
    match expr {
        QueryExpr::All | QueryExpr::Term(_) | QueryExpr::Phrase(_) | QueryExpr::Prefix(_) => false,
        QueryExpr::Wildcard(_) | QueryExpr::Fuzzy { .. } | QueryExpr::Regex(_) => true,
        QueryExpr::And(a, b) | QueryExpr::Or(a, b) => {
            expr_needs_post_filter(a) || expr_needs_post_filter(b)
        }
        QueryExpr::Not(inner) => expr_needs_post_filter(inner),
    }
}

fn find_trigram_hint(expr: &QueryExpr) -> Option<String> {
    match expr {
        QueryExpr::Wildcard(pat) => {
            let best = pat
                .split(['*', '?'])
                .filter(|s| s.chars().count() >= 3)
                .max_by_key(|s| s.len())?;
            Some(best.to_owned())
        }
        QueryExpr::Fuzzy { term, .. } if term.chars().count() >= 4 => {
            Some(term.chars().take(3).collect())
        }
        QueryExpr::And(a, b) => find_trigram_hint(a).or_else(|| find_trigram_hint(b)),
        _ => None,
    }
}

/// Translates `expr` into an SQLite FTS5 `MATCH` string if possible.
///
/// Pure `NOT` expressions or pure regex/fuzzy expressions return `None` so callers query candidates
/// via SQL filters and apply [`ParsedQuery::matches_hit`] in memory.
#[must_use]
pub fn build_fts5_expr(expr: &QueryExpr) -> Option<String> {
    match expr {
        QueryExpr::All | QueryExpr::Wildcard(_) | QueryExpr::Fuzzy { .. } | QueryExpr::Regex(_) => {
            None
        }
        QueryExpr::Term(t) => Some(escape_fts5_term(t)),
        QueryExpr::Phrase(p) => Some(escape_fts5_term(p)),
        QueryExpr::Prefix(p) => Some(format!("{}*", escape_fts5_term(p))),
        QueryExpr::And(a, b) => match (build_fts5_expr(a), build_fts5_expr(b)) {
            (Some(lhs), Some(rhs)) => Some(format!("({lhs} AND {rhs})")),
            (Some(lhs), None) => Some(lhs),
            (None, Some(rhs)) => Some(rhs),
            (None, None) => None,
        },
        QueryExpr::Or(a, b) => match (build_fts5_expr(a), build_fts5_expr(b)) {
            (Some(lhs), Some(rhs)) => Some(format!("({lhs} OR {rhs})")),
            _ => None,
        },
        QueryExpr::Not(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Word(String),
    Phrase(String),
    Regex(String),
    Field { key: String, value: String },
    And,
    Or,
    Not,
    LParen,
    RParen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    pos: usize,
    raw: String,
    kind: TokenKind,
}

fn tokenize(input: &str) -> Result<Vec<Token>, QueryParseError> {
    let mut tokens = Vec::new();
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        let (pos, ch) = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '(' {
            tokens.push(Token {
                pos,
                raw: "(".to_owned(),
                kind: TokenKind::LParen,
            });
            i += 1;
            continue;
        }
        if ch == ')' {
            tokens.push(Token {
                pos,
                raw: ")".to_owned(),
                kind: TokenKind::RParen,
            });
            i += 1;
            continue;
        }
        if ch == '"' {
            let (phrase, next_i) = read_quoted(&chars, i, pos)?;
            tokens.push(Token {
                pos,
                raw: format!("\"{phrase}\""),
                kind: TokenKind::Phrase(phrase),
            });
            i = next_i;
            continue;
        }
        if ch == '/' {
            let (pattern, next_i) = read_regex(&chars, i, pos)?;
            tokens.push(Token {
                pos,
                raw: format!("/{pattern}/"),
                kind: TokenKind::Regex(pattern),
            });
            i = next_i;
            continue;
        }
        if ch == '-' && i + 1 < chars.len() && !chars[i + 1].1.is_whitespace() {
            tokens.push(Token {
                pos,
                raw: "-".to_owned(),
                kind: TokenKind::Not,
            });
            i += 1;
            continue;
        }

        // Read bare word or key:value / key:"quoted value".
        let mut word = String::new();
        while i < chars.len() {
            let (_, c) = chars[i];
            if c.is_whitespace() || c == '(' || c == ')' {
                break;
            }
            if c == ':' && !word.is_empty() && !word.contains(':') {
                let key = word.to_ascii_lowercase();
                i += 1; // skip ':'
                if i < chars.len() && chars[i].1 == '"' {
                    let (val, next_i) = read_quoted(&chars, i, chars[i].0)?;
                    i = next_i;
                    tokens.push(Token {
                        pos,
                        raw: format!("{key}:\"{val}\""),
                        kind: TokenKind::Field { key, value: val },
                    });
                } else {
                    let mut val = String::new();
                    while i < chars.len() && !chars[i].1.is_whitespace() && chars[i].1 != ')' {
                        val.push(chars[i].1);
                        i += 1;
                    }
                    tokens.push(Token {
                        pos,
                        raw: format!("{key}:{val}"),
                        kind: TokenKind::Field { key, value: val },
                    });
                }
                word.clear();
                break;
            }
            word.push(c);
            i += 1;
        }
        if !word.is_empty() {
            let kind = match word.as_str() {
                "AND" => TokenKind::And,
                "OR" => TokenKind::Or,
                "NOT" => TokenKind::Not,
                _ => TokenKind::Word(word.clone()),
            };
            tokens.push(Token {
                pos,
                raw: word,
                kind,
            });
        }
    }

    Ok(tokens)
}

fn read_quoted(
    chars: &[(usize, char)],
    start_idx: usize,
    start_pos: usize,
) -> Result<(String, usize), QueryParseError> {
    let mut out = String::new();
    let mut i = start_idx + 1;
    while i < chars.len() {
        let (_, c) = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1].1);
            i += 2;
            continue;
        }
        if c == '"' {
            return Ok((out, i + 1));
        }
        out.push(c);
        i += 1;
    }
    Err(QueryParseError {
        position: start_pos,
        message: "unclosed quote `\"` in search query".to_owned(),
    })
}

fn read_regex(
    chars: &[(usize, char)],
    start_idx: usize,
    start_pos: usize,
) -> Result<(String, usize), QueryParseError> {
    let mut out = String::new();
    let mut i = start_idx + 1;
    while i < chars.len() {
        let (_, c) = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push('\\');
            out.push(chars[i + 1].1);
            i += 2;
            continue;
        }
        if c == '/' {
            if out.is_empty() {
                return Err(QueryParseError {
                    position: start_pos,
                    message: "empty regex `//` is not allowed".to_owned(),
                });
            }
            return Ok((out, i + 1));
        }
        out.push(c);
        i += 1;
    }
    Err(QueryParseError {
        position: start_pos,
        message: "unclosed regex `/` — add a closing `/`".to_owned(),
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let tok = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(tok)
    }

    fn parse_or(&mut self) -> Result<QueryExpr, QueryParseError> {
        let mut lhs = self.parse_and()?;
        while let Some(tok) = self.peek() {
            if tok.kind == TokenKind::Or {
                let op_pos = tok.pos;
                self.next();
                if self.peek().is_none() {
                    return Err(QueryParseError {
                        position: op_pos,
                        message: "`OR` operator is missing a right-hand term".to_owned(),
                    });
                }
                let rhs = self.parse_and()?;
                lhs = QueryExpr::Or(Box::new(lhs), Box::new(rhs));
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<QueryExpr, QueryParseError> {
        let mut lhs = self.parse_not()?;
        while let Some(tok) = self.peek() {
            match tok.kind {
                TokenKind::And => {
                    let op_pos = tok.pos;
                    self.next();
                    if self.peek().is_none() {
                        return Err(QueryParseError {
                            position: op_pos,
                            message: "`AND` operator is missing a right-hand term".to_owned(),
                        });
                    }
                    let rhs = self.parse_not()?;
                    lhs = QueryExpr::And(Box::new(lhs), Box::new(rhs));
                }
                TokenKind::Or | TokenKind::RParen => break,
                _ => {
                    let rhs = self.parse_not()?;
                    lhs = QueryExpr::And(Box::new(lhs), Box::new(rhs));
                }
            }
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<QueryExpr, QueryParseError> {
        if let Some(tok) = self.peek()
            && tok.kind == TokenKind::Not
        {
            let op_pos = tok.pos;
            self.next();
            if self.peek().is_none() {
                return Err(QueryParseError {
                    position: op_pos,
                    message: "`NOT` operator is missing a term".to_owned(),
                });
            }
            let inner = self.parse_not()?;
            return Ok(QueryExpr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<QueryExpr, QueryParseError> {
        let Some(tok) = self.next() else {
            return Err(QueryParseError {
                position: 0,
                message: "unexpected end of search query".to_owned(),
            });
        };
        match &tok.kind {
            TokenKind::LParen => {
                if self.peek().is_some_and(|t| t.kind == TokenKind::RParen) {
                    return Err(QueryParseError {
                        position: tok.pos,
                        message: "empty parentheses `()` in search query".to_owned(),
                    });
                }
                let inner = self.parse_or()?;
                match self.next() {
                    Some(close) if close.kind == TokenKind::RParen => Ok(inner),
                    _ => Err(QueryParseError {
                        position: tok.pos,
                        message: "unclosed `(` — add a matching `)`".to_owned(),
                    }),
                }
            }
            TokenKind::Phrase(p) => {
                if p.trim().is_empty() {
                    return Err(QueryParseError {
                        position: tok.pos,
                        message: "empty quoted phrase `\"\"`".to_owned(),
                    });
                }
                Ok(QueryExpr::Phrase(p.clone()))
            }
            TokenKind::Regex(pattern) => {
                let compiled = RegexPattern::parse(pattern).map_err(|msg| QueryParseError {
                    position: tok.pos,
                    message: format!("invalid regex `/{pattern}/`: {msg}"),
                })?;
                Ok(QueryExpr::Regex(compiled))
            }
            TokenKind::Word(word) => parse_word_expr(word, tok.pos),
            TokenKind::And
            | TokenKind::Or
            | TokenKind::RParen
            | TokenKind::Not
            | TokenKind::Field { .. } => Err(QueryParseError {
                position: tok.pos,
                message: format!("unexpected token `{}`", tok.raw),
            }),
        }
    }
}

fn parse_word_expr(word: &str, pos: usize) -> Result<QueryExpr, QueryParseError> {
    if let Some((base, dist_str)) = word.rsplit_once('~') {
        if base.is_empty() {
            return Err(QueryParseError {
                position: pos,
                message: "fuzzy operator `~` needs a word before it".to_owned(),
            });
        }
        let max_edits = if dist_str.is_empty() {
            1
        } else {
            let n = dist_str.parse::<usize>().map_err(|_| QueryParseError {
                position: pos,
                message: format!("invalid fuzzy distance `{dist_str}`; expected an integer 1..=3"),
            })?;
            n.clamp(1, 3)
        };
        return Ok(QueryExpr::Fuzzy {
            term: base.to_owned(),
            max_edits,
        });
    }
    if word.contains('*') || word.contains('?') {
        let stripped = word.trim_end_matches('*');
        if !stripped.contains('*')
            && !stripped.contains('?')
            && !stripped.is_empty()
            && word.ends_with('*')
        {
            return Ok(QueryExpr::Prefix(stripped.to_owned()));
        }
        return Ok(QueryExpr::Wildcard(word.to_owned()));
    }
    Ok(QueryExpr::Term(word.to_owned()))
}

fn apply_field_filter(
    filters: &mut SearchFilters,
    key: &str,
    value: &str,
    pos: usize,
) -> Result<(), QueryParseError> {
    if value.trim().is_empty() {
        return Err(QueryParseError {
            position: pos,
            message: format!("operator `{key}:` requires a value"),
        });
    }
    match key {
        "author" => filters.author = Some(value.to_owned()),
        "tag" => filters.tags.push(value.to_owned()),
        "color" => filters.color = Some(normalize_color(value)),
        "after" => {
            let ts = parse_date_or_timestamp(value, DayBoundary::StartOfDay).ok_or_else(|| {
                QueryParseError {
                    position: pos,
                    message: format!(
                        "invalid date `{value}` for `after:` (expected YYYY-MM-DD or epoch seconds)"
                    ),
                }
            })?;
            filters.after = Some(ts);
        }
        "before" => {
            let ts = parse_date_or_timestamp(value, DayBoundary::EndOfDay).ok_or_else(|| {
                QueryParseError {
                    position: pos,
                    message: format!(
                        "invalid date `{value}` for `before:` (expected YYYY-MM-DD or epoch seconds)"
                    ),
                }
            })?;
            filters.before = Some(ts);
        }
        "page" => {
            let pf = parse_page_filter(value).ok_or_else(|| QueryParseError {
                position: pos,
                message: format!("invalid page filter `{value}` (expected e.g. `page:5`, `page:>100`, `page:2..8`)"),
            })?;
            let (min_p, max_p) = pf.to_zero_based_bounds();
            if let Some(m) = min_p {
                filters.page_min = Some(filters.page_min.map_or(m, |prev| prev.max(m)));
            }
            if let Some(m) = max_p {
                filters.page_max = Some(filters.page_max.map_or(m, |prev| prev.min(m)));
            }
        }
        "kind" | "in" => {
            let kind = SourceKind::from_name(&value.to_ascii_lowercase()).ok_or_else(|| {
                QueryParseError {
                    position: pos,
                    message: format!(
                        "unknown kind `{value}` (expected page, annotation, handwriting, transcript, or note)"
                    ),
                }
            })?;
            if !filters.source_kinds.contains(&kind) {
                filters.source_kinds.push(kind);
            }
        }
        "path" | "folder" => filters.path_prefix = Some(value.to_owned()),
        "file" => filters.file_path = Some(PathBuf::from(value)),
        _ => {
            return Err(QueryParseError {
                position: pos,
                message: format!(
                    "unknown search operator `{key}:` (supported: author, tag, color, after, before, page, kind, path, file)"
                ),
            });
        }
    }
    Ok(())
}

fn normalize_color(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "yellow" => "#ffff00".to_owned(),
        "red" => "#ff0000".to_owned(),
        "green" => "#00ff00".to_owned(),
        "blue" => "#0000ff".to_owned(),
        "orange" => "#ffa500".to_owned(),
        "purple" => "#800080".to_owned(),
        "pink" => "#ffc0cb".to_owned(),
        other => other.to_owned(),
    }
}

/// Which edge of a calendar date a `YYYY-MM-DD` filter resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DayBoundary {
    /// Beginning of the UTC day (`00:00:00`).
    StartOfDay,
    /// End of the UTC day (`23:59:59`).
    EndOfDay,
}

/// Parses `YYYY-MM-DD` (at `boundary` UTC) or a raw integer timestamp.
#[must_use]
pub fn parse_date_or_timestamp(value: &str, boundary: DayBoundary) -> Option<i64> {
    let trimmed = value.trim();
    if let Ok(ts) = trimmed.parse::<i64>() {
        return Some(ts);
    }
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    let max_day = days_in_month(year, month);
    if day > max_day {
        return None;
    }
    let days = days_since_unix_epoch(year, month, day);
    let offset = match boundary {
        DayBoundary::StartOfDay => 0,
        DayBoundary::EndOfDay => 86_399,
    };
    let secs = days * 86_400 + offset;
    Some(secs)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_since_unix_epoch(year: i64, month: u32, day: u32) -> i64 {
    let mut total = 0_i64;
    for y in 1970..year {
        total += if is_leap_year(y) { 366 } else { 365 };
    }
    for m in 1..month {
        total += i64::from(days_in_month(year, m));
    }
    total + i64::from(day - 1)
}

/// Parses `page:5`, `page:>100`, `page:>=100`, `page:<10`, `page:<=10`, or `page:2..8`.
#[must_use]
pub fn parse_page_filter(value: &str) -> Option<PageFilter> {
    let s = value.trim();
    if let Some((a, b)) = s.split_once("..") {
        let lo = a.parse::<usize>().ok()?.max(1);
        let hi = b.parse::<usize>().ok()?.max(1);
        return (lo <= hi).then_some(PageFilter::Range(lo, hi));
    }
    if let Some(rest) = s.strip_prefix(">=") {
        return Some(PageFilter::GreaterOrEqual(
            rest.parse::<usize>().ok()?.max(1),
        ));
    }
    if let Some(rest) = s.strip_prefix("<=") {
        return Some(PageFilter::LessOrEqual(rest.parse::<usize>().ok()?.max(1)));
    }
    if let Some(rest) = s.strip_prefix('>') {
        return Some(PageFilter::GreaterThan(rest.parse::<usize>().ok()?.max(1)));
    }
    if let Some(rest) = s.strip_prefix('<') {
        let p = rest.parse::<usize>().ok()?;
        return (p >= 2).then_some(PageFilter::LessThan(p));
    }
    Some(PageFilter::Exact(s.parse::<usize>().ok()?.max(1)))
}

/// Saves a named search / smart folder (plan F.5).
///
/// # Errors
///
/// Returns [`QueryParseError`] if `query` is invalid, or [`ecclesia_store::Error`] if SQLite fails.
pub fn create_saved_search(
    tx: &Transaction<'_>,
    name: &str,
    query: &str,
    created_at: i64,
) -> Result<i64, SavedSearchError> {
    let _ = ParsedQuery::parse(query)?;
    let id = ecclesia_store::fts::save_search(tx, name, query, created_at)?;
    Ok(id)
}

/// Lists all saved searches from `library.db` (plan F.5).
///
/// # Errors
///
/// SQLite failed.
pub fn list_saved_searches(conn: &Connection) -> ecclesia_store::Result<Vec<SavedSearch>> {
    ecclesia_store::fts::list_searches(conn)
}

/// Error returned when creating or evaluating a saved search.
#[derive(Debug)]
#[non_exhaustive]
pub enum SavedSearchError {
    /// The query string failed to parse.
    Parse(QueryParseError),
    /// SQLite failed.
    Store(ecclesia_store::Error),
}

impl fmt::Display for SavedSearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => err.fmt(f),
            Self::Store(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for SavedSearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(err) => Some(err),
            Self::Store(err) => Some(err),
        }
    }
}

impl From<QueryParseError> for SavedSearchError {
    fn from(err: QueryParseError) -> Self {
        Self::Parse(err)
    }
}

impl From<ecclesia_store::Error> for SavedSearchError {
    fn from(err: ecclesia_store::Error) -> Self {
        Self::Store(err)
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut dp = vec![false; t.len() + 1];
    dp[0] = true;
    for pc in p {
        let mut next = vec![false; t.len() + 1];
        if pc == '*' {
            let mut seen = false;
            for j in 0..=t.len() {
                seen = seen || dp[j];
                next[j] = seen;
            }
        } else {
            for j in 0..t.len() {
                if dp[j] && (pc == '?' || pc == t[j]) {
                    next[j + 1] = true;
                }
            }
        }
        dp = next;
    }
    dp[t.len()]
}

fn levenshtein_within(a: &str, b: &str, max_edits: usize) -> bool {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    if av.len().abs_diff(bv.len()) > max_edits {
        return false;
    }
    let mut prev: Vec<usize> = (0..=bv.len()).collect();
    let mut curr = vec![0; bv.len() + 1];
    for (i, &ac) in av.iter().enumerate() {
        curr[0] = i + 1;
        let mut row_min = curr[0];
        for (j, &bc) in bv.iter().enumerate() {
            let cost = usize::from(ac != bc);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
            row_min = row_min.min(curr[j + 1]);
        }
        if row_min > max_edits {
            return false;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[bv.len()] <= max_edits
}

/// Lightweight compiled regular expression for `/pattern/` queries (plan F.4).
///
/// Supports literals, `.`, `^`, `$`, character classes (`[a-z0-9]`, `[^...]`),
/// shorthand classes (`\d`, `\w`, `\s`), alternation (`|`), grouping (`(...)`),
/// and quantifiers (`*`, `+`, `?`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexPattern {
    raw: String,
    branches: Vec<Vec<RegexNode>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegexNode {
    atom: RegexAtom,
    quant: Quantifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quantifier {
    One,
    ZeroOrMore,
    OneOrMore,
    ZeroOrOne,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RegexAtom {
    StartAnchor,
    EndAnchor,
    AnyChar,
    Literal(char),
    Digit,
    Word,
    Space,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Group(Vec<Vec<RegexNode>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

/// Error returned when a `/regex/` pattern in a search query is invalid (plan F.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexParseError(String);

impl fmt::Display for RegexParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid regex: {}", self.0)
    }
}

impl std::error::Error for RegexParseError {}

impl RegexPattern {
    /// Parses a regex pattern string.
    ///
    /// # Errors
    ///
    /// Returns [`RegexParseError`] describing syntax errors such as unclosed groups, unclosed `[...]`,
    /// or trailing escapes.
    pub fn parse(pattern: &str) -> Result<Self, RegexParseError> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut idx = 0;
        let branches = parse_branches(&chars, &mut idx, false).map_err(RegexParseError)?;
        Ok(Self {
            raw: pattern.to_owned(),
            branches,
        })
    }

    /// Returns the raw pattern string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns `true` if the pattern matches anywhere inside `haystack`.
    #[must_use]
    pub fn is_match(&self, haystack: &str) -> bool {
        let chars: Vec<char> = haystack.chars().collect();
        for start in 0..=chars.len() {
            if match_branches(&self.branches, &chars, start).is_some() {
                return true;
            }
        }
        false
    }
}

fn parse_branches(
    chars: &[char],
    idx: &mut usize,
    in_group: bool,
) -> Result<Vec<Vec<RegexNode>>, String> {
    let mut branches = vec![Vec::new()];
    while *idx < chars.len() {
        let ch = chars[*idx];
        if in_group && ch == ')' {
            *idx += 1;
            return Ok(branches);
        }
        if ch == '|' {
            *idx += 1;
            branches.push(Vec::new());
            continue;
        }
        let atom = match ch {
            '^' => {
                *idx += 1;
                RegexAtom::StartAnchor
            }
            '$' => {
                *idx += 1;
                RegexAtom::EndAnchor
            }
            '.' => {
                *idx += 1;
                RegexAtom::AnyChar
            }
            '(' => {
                *idx += 1;
                RegexAtom::Group(parse_branches(chars, idx, true)?)
            }
            ')' => return Err("unmatched `)`".to_owned()),
            '[' => parse_class(chars, idx)?,
            '\\' => {
                *idx += 1;
                let Some(&esc) = chars.get(*idx) else {
                    return Err("trailing `\\`".to_owned());
                };
                *idx += 1;
                match esc {
                    'd' => RegexAtom::Digit,
                    'w' => RegexAtom::Word,
                    's' => RegexAtom::Space,
                    other => RegexAtom::Literal(other),
                }
            }
            '*' | '+' | '?' => return Err(format!("nothing before quantifier `{ch}`")),
            other => {
                *idx += 1;
                RegexAtom::Literal(other)
            }
        };
        let quant = match chars.get(*idx) {
            Some('*') => {
                *idx += 1;
                Quantifier::ZeroOrMore
            }
            Some('+') => {
                *idx += 1;
                Quantifier::OneOrMore
            }
            Some('?') => {
                *idx += 1;
                Quantifier::ZeroOrOne
            }
            _ => Quantifier::One,
        };
        if let Some(last) = branches.last_mut() {
            last.push(RegexNode { atom, quant });
        }
    }
    if in_group {
        return Err("unclosed `(`".to_owned());
    }
    Ok(branches)
}

fn parse_class(chars: &[char], idx: &mut usize) -> Result<RegexAtom, String> {
    *idx += 1; // skip '['
    let negated = if chars.get(*idx) == Some(&'^') {
        *idx += 1;
        true
    } else {
        false
    };
    let mut items = Vec::new();
    while *idx < chars.len() {
        let ch = chars[*idx];
        if ch == ']' {
            *idx += 1;
            return Ok(RegexAtom::Class { negated, items });
        }
        let first = if ch == '\\' {
            *idx += 1;
            let Some(&esc) = chars.get(*idx) else {
                return Err("unclosed `[`".to_owned());
            };
            *idx += 1;
            esc
        } else {
            *idx += 1;
            ch
        };
        if chars.get(*idx) == Some(&'-') && chars.get(*idx + 1).is_some_and(|&n| n != ']') {
            *idx += 1;
            let end = chars[*idx];
            *idx += 1;
            items.push(ClassItem::Range(first, end));
        } else {
            items.push(ClassItem::Single(first));
        }
    }
    Err("unclosed `[`".to_owned())
}

fn match_branches(branches: &[Vec<RegexNode>], chars: &[char], pos: usize) -> Option<usize> {
    for branch in branches {
        if let Some(end) = match_seq(branch, chars, pos) {
            return Some(end);
        }
    }
    None
}

fn match_seq(seq: &[RegexNode], chars: &[char], pos: usize) -> Option<usize> {
    let Some((node, rest)) = seq.split_first() else {
        return Some(pos);
    };
    match node.quant {
        Quantifier::One => {
            let next = match_atom(&node.atom, chars, pos)?;
            match_seq(rest, chars, next)
        }
        Quantifier::ZeroOrOne => {
            if let Some(next) = match_atom(&node.atom, chars, pos)
                && let Some(end) = match_seq(rest, chars, next)
            {
                return Some(end);
            }
            match_seq(rest, chars, pos)
        }
        Quantifier::ZeroOrMore => match_repeat(&node.atom, rest, chars, pos),
        Quantifier::OneOrMore => {
            let next = match_atom(&node.atom, chars, pos)?;
            match_repeat(&node.atom, rest, chars, next)
        }
    }
}

fn match_repeat(atom: &RegexAtom, rest: &[RegexNode], chars: &[char], pos: usize) -> Option<usize> {
    let mut positions = vec![pos];
    let mut cur = pos;
    while let Some(next) = match_atom(atom, chars, cur) {
        if next == cur {
            break;
        }
        positions.push(next);
        cur = next;
    }
    for &p in positions.iter().rev() {
        if let Some(end) = match_seq(rest, chars, p) {
            return Some(end);
        }
    }
    None
}

fn match_atom(atom: &RegexAtom, chars: &[char], pos: usize) -> Option<usize> {
    match atom {
        RegexAtom::StartAnchor => (pos == 0).then_some(pos),
        RegexAtom::EndAnchor => (pos == chars.len()).then_some(pos),
        RegexAtom::AnyChar => chars.get(pos).filter(|&&c| c != '\n').map(|_| pos + 1),
        RegexAtom::Literal(expected) => {
            chars.get(pos).filter(|&&c| c == *expected).map(|_| pos + 1)
        }
        RegexAtom::Digit => chars
            .get(pos)
            .filter(|c| c.is_ascii_digit())
            .map(|_| pos + 1),
        RegexAtom::Word => chars
            .get(pos)
            .filter(|c| c.is_alphanumeric() || **c == '_')
            .map(|_| pos + 1),
        RegexAtom::Space => chars
            .get(pos)
            .filter(|c| c.is_whitespace())
            .map(|_| pos + 1),
        RegexAtom::Class { negated, items } => {
            let &ch = chars.get(pos)?;
            let matched = items.iter().any(|item| match item {
                ClassItem::Single(c) => ch == *c,
                ClassItem::Range(a, b) => (*a..=*b).contains(&ch),
            });
            (matched != *negated).then_some(pos + 1)
        }
        RegexAtom::Group(branches) => match_branches(branches, chars, pos),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_operators_and_translates_to_fts5() {
        let q = ParsedQuery::parse(
            r#"author:"Ada Lovelace" tag:Method color:yellow after:2025-01-15 before:2026-12-31 page:>100 kind:annotation ("neural net" OR transformer*) NOT draft"#,
        )
        .expect("parse");

        assert_eq!(q.filters.author.as_deref(), Some("Ada Lovelace"));
        assert_eq!(q.filters.tags, ["Method"]);
        assert_eq!(q.filters.color.as_deref(), Some("#ffff00"));
        assert!(q.filters.after.is_some());
        assert!(q.filters.before.is_some());
        assert!(q.filters.after.expect("after") < q.filters.before.expect("before"));
        assert_eq!(q.filters.page_min, Some(100));
        assert_eq!(q.filters.source_kinds, [SourceKind::Annotation]);
        let fts = q.fts_match.expect("fts5");
        assert!(fts.contains("neural net"));
        assert!(fts.contains("transformer\"*"));
    }

    #[test]
    fn fuzzy_wildcard_and_regex_evaluate_accurately() {
        let hit = FtsHit {
            id: 1,
            path: None,
            source_kind: SourceKind::Page,
            source_id: String::new(),
            page: 0,
            author: String::new(),
            color: String::new(),
            tags: Vec::new(),
            created_at: 0,
            quads_json: "[]".to_owned(),
            title: "Paper".to_owned(),
            snippet: String::new(),
            body: "DOI 10.1145/3295222 on transformer architecture".to_owned(),
            rank: 0.0,
        };

        let fuzzy = ParsedQuery::parse("transfromer~2").expect("fuzzy");
        assert!(fuzzy.matches_hit(&hit));
        assert!(
            !ParsedQuery::parse("completelydifferent~1")
                .expect("fuzzy miss")
                .matches_hit(&hit)
        );

        let wild = ParsedQuery::parse("*form*").expect("wild");
        assert!(wild.matches_hit(&hit));

        let regex = ParsedQuery::parse(r"/10\.\d+\/\d+/").expect("regex");
        assert!(regex.matches_hit(&hit));
    }

    #[test]
    fn malformed_queries_produce_helpful_errors() {
        for bad in [
            "\"unclosed phrase",
            "/unclosed",
            "author:",
            "after:not-a-date",
            "page:abc",
            "(foo OR bar",
            "foo AND",
            "unknownop:123",
            "()",
        ] {
            let err = ParsedQuery::parse(bad).expect_err(&format!("should fail: {bad}"));
            assert!(!err.message.is_empty(), "helpful message for {bad}");
        }
    }
}
