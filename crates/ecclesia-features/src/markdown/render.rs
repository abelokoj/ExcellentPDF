//! GFM-compatible Markdown to HTML renderer with KaTeX math and Mermaid support (plan N.14).
//!
//! Emits headings with GitHub anchor `id="..."` matching [`crate::markdown::toc`], preserves
//! display (`$$...$$`) and inline (`$...$`) TeX expressions for KaTeX, transforms ` ```mermaid `
//! code fences into `<div class="mermaid">...</div>` containers, and supports GFM tables,
//! task lists, strikethrough, autolinks, and GFM tag filtering.

use std::fmt::Write as _;

use crate::markdown::toc::{
    Slugger, TocHeading, is_closing_fence, parse_atx_heading, parse_opening_fence,
    parse_setext_underline, strip_inline_markdown,
};

/// Options controlling Markdown HTML rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RenderOptions {
    /// Whether to emit `<div class="mermaid">` blocks for `mermaid` fences (enabled in WebView mode).
    pub enable_mermaid: bool,
    /// Whether to emit `<span/div class="math">` markers for `$...$` / `$$...$$` (enabled in WebView mode).
    pub enable_math: bool,
}

impl RenderOptions {
    /// Full interactive preview options (KaTeX math + Mermaid diagrams enabled).
    #[must_use]
    pub fn rich_preview() -> Self {
        Self {
            enable_mermaid: true,
            enable_math: true,
        }
    }

    /// Plain fallback options (no KaTeX/Mermaid scripts, suitable for MuPDF HTML rendering).
    #[must_use]
    pub fn plain_fallback() -> Self {
        Self {
            enable_mermaid: false,
            enable_math: false,
        }
    }
}

/// Output of rendering a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMarkdown {
    /// Rendered HTML body fragment.
    pub html: String,
    /// Extracted heading table of contents whose slugs match every heading `id="..."` in `html`.
    pub headings: Vec<TocHeading>,
    /// Whether the document contains at least one math expression.
    pub has_math: bool,
    /// Whether the document contains at least one Mermaid diagram fence.
    pub has_mermaid: bool,
}

/// Renders `markdown` to GFM HTML with heading anchors, math blocks, and Mermaid diagrams.
#[must_use]
pub fn render_gfm_html(markdown: &str, options: RenderOptions) -> RenderedMarkdown {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut html = String::with_capacity(markdown.len() * 2);
    let mut headings = Vec::new();
    let mut slugger = Slugger::new();
    let mut has_math = false;
    let mut has_mermaid = false;

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 1. Display math block `$$ ... $$`
        if let Some(after_open) = trimmed.strip_prefix("$$") {
            has_math = true;
            if let Some(inner) = after_open.strip_suffix("$$")
                && !after_open.is_empty()
            {
                emit_display_math(&mut html, inner.trim(), options.enable_math, i + 1);
                i += 1;
                continue;
            }
            let start_line = i + 1;
            let mut body = String::new();
            let first_rest = after_open.trim();
            if !first_rest.is_empty() {
                body.push_str(first_rest);
                body.push('\n');
            }
            i += 1;
            while i < lines.len() {
                if lines[i].trim() == "$$" {
                    i += 1;
                    break;
                }
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            emit_display_math(&mut html, body.trim(), options.enable_math, start_line);
            continue;
        }

        // 2. Fenced code block (` ``` ` or `~~~`) including Mermaid diagrams.
        if let Some((fch, flen)) = parse_opening_fence(line.trim_start()) {
            let start_line = i + 1;
            let info = line.trim_start()[flen..].trim();
            let lang = info
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut code = String::new();
            i += 1;
            while i < lines.len() {
                if is_closing_fence(lines[i].trim_start(), fch, flen) {
                    i += 1;
                    break;
                }
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            if lang == "mermaid" {
                has_mermaid = true;
                if options.enable_mermaid {
                    let _ = writeln!(
                        html,
                        "<div class=\"mermaid\" data-line=\"{start_line}\">{}</div>",
                        escape_html(code.trim_end())
                    );
                } else {
                    let _ = writeln!(
                        html,
                        "<pre data-line=\"{start_line}\"><code class=\"language-mermaid\">{}</code></pre>",
                        escape_html(&code)
                    );
                }
            } else if lang.is_empty() {
                let _ = writeln!(
                    html,
                    "<pre data-line=\"{start_line}\"><code>{}</code></pre>",
                    escape_html(&code)
                );
            } else {
                let _ = writeln!(
                    html,
                    "<pre data-line=\"{start_line}\"><code class=\"language-{}\">{}</code></pre>",
                    escape_html(&lang),
                    escape_html(&code)
                );
            }
            continue;
        }

        // 3. ATX Heading
        if let Some((level, raw_title)) = parse_atx_heading(line) {
            let plain = strip_inline_markdown(raw_title);
            let slug = slugger.slug(&plain);
            let rendered_title = render_inline(raw_title, options.enable_math, &mut has_math);
            let _ = writeln!(
                html,
                "<h{level} id=\"{}\" data-line=\"{}\">{rendered_title}</h{level}>",
                escape_html(&slug),
                i + 1
            );
            headings.push(TocHeading {
                level,
                title: plain,
                slug,
                line: i + 1,
            });
            i += 1;
            continue;
        }

        // 4. Setext Heading
        if i + 1 < lines.len()
            && let Some(level) = parse_setext_underline(lines[i + 1])
        {
            let plain = strip_inline_markdown(trimmed);
            let slug = slugger.slug(&plain);
            let rendered_title = render_inline(trimmed, options.enable_math, &mut has_math);
            let _ = writeln!(
                html,
                "<h{level} id=\"{}\" data-line=\"{}\">{rendered_title}</h{level}>",
                escape_html(&slug),
                i + 1
            );
            headings.push(TocHeading {
                level,
                title: plain,
                slug,
                line: i + 1,
            });
            i += 2;
            continue;
        }

        // 5. Thematic break / horizontal rule
        if is_horizontal_rule(trimmed) {
            html.push_str("<hr />\n");
            i += 1;
            continue;
        }

        // 6. GFM Table (header row + delimiter row)
        if i + 1 < lines.len()
            && line.contains('|')
            && let Some(aligns) = parse_table_delimiter(lines[i + 1])
        {
            let start_line = i + 1;
            let header_cells = split_table_row(line);
            let _ = writeln!(html, "<table data-line=\"{start_line}\"><thead><tr>");
            for (col, cell) in header_cells.iter().enumerate() {
                let align_attr = aligns.get(col).copied().unwrap_or("");
                let cell_html = render_inline(cell, options.enable_math, &mut has_math);
                let _ = write!(html, "<th{align_attr}>{cell_html}</th>");
            }
            html.push_str("</tr></thead><tbody>\n");
            i += 2;
            while i < lines.len() && !lines[i].trim().is_empty() && lines[i].contains('|') {
                let cells = split_table_row(lines[i]);
                html.push_str("<tr>");
                for (col, cell) in cells.iter().enumerate() {
                    let align_attr = aligns.get(col).copied().unwrap_or("");
                    let cell_html = render_inline(cell, options.enable_math, &mut has_math);
                    let _ = write!(html, "<td{align_attr}>{cell_html}</td>");
                }
                html.push_str("</tr>\n");
                i += 1;
            }
            html.push_str("</tbody></table>\n");
            continue;
        }

        // 7. Blockquote
        if trimmed.starts_with('>') {
            let start_line = i + 1;
            let mut inner = String::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let stripped = lines[i].trim_start().strip_prefix('>').unwrap_or("");
                let stripped = stripped.strip_prefix(' ').unwrap_or(stripped);
                inner.push_str(stripped);
                inner.push('\n');
                i += 1;
            }
            let nested = render_gfm_html(&inner, options);
            has_math |= nested.has_math;
            has_mermaid |= nested.has_mermaid;
            let _ = writeln!(
                html,
                "<blockquote data-line=\"{start_line}\">\n{}</blockquote>",
                nested.html
            );
            continue;
        }

        // 8. Unordered / Task list
        if parse_ul_item(trimmed).is_some() {
            let start_line = i + 1;
            let _ = writeln!(html, "<ul data-line=\"{start_line}\">");
            while i < lines.len() {
                let Some(item) = parse_ul_item(lines[i].trim()) else {
                    break;
                };
                if let Some(rest) = item
                    .strip_prefix("[x] ")
                    .or_else(|| item.strip_prefix("[X] "))
                {
                    let body = render_inline(rest, options.enable_math, &mut has_math);
                    let _ = writeln!(
                        html,
                        "<li class=\"task-list-item\"><input type=\"checkbox\" checked disabled /> {body}</li>"
                    );
                } else if let Some(rest) = item.strip_prefix("[ ] ") {
                    let body = render_inline(rest, options.enable_math, &mut has_math);
                    let _ = writeln!(
                        html,
                        "<li class=\"task-list-item\"><input type=\"checkbox\" disabled /> {body}</li>"
                    );
                } else {
                    let body = render_inline(item, options.enable_math, &mut has_math);
                    let _ = writeln!(html, "<li>{body}</li>");
                }
                i += 1;
            }
            html.push_str("</ul>\n");
            continue;
        }

        // 9. Ordered list
        if parse_ol_item(trimmed).is_some() {
            let start_line = i + 1;
            let _ = writeln!(html, "<ol data-line=\"{start_line}\">");
            while i < lines.len() {
                let Some(item) = parse_ol_item(lines[i].trim()) else {
                    break;
                };
                let body = render_inline(item, options.enable_math, &mut has_math);
                let _ = writeln!(html, "<li>{body}</li>");
                i += 1;
            }
            html.push_str("</ol>\n");
            continue;
        }

        // 10. Paragraph (collect until blank line or next block opener)
        let start_line = i + 1;
        let mut para = Vec::new();
        while i < lines.len() {
            let cur = lines[i];
            let cur_trim = cur.trim();
            if cur_trim.is_empty()
                || parse_opening_fence(cur.trim_start()).is_some()
                || parse_atx_heading(cur).is_some()
                || cur_trim.starts_with("$$")
                || cur_trim.starts_with('>')
                || is_horizontal_rule(cur_trim)
                || parse_ul_item(cur_trim).is_some()
                || parse_ol_item(cur_trim).is_some()
            {
                break;
            }
            para.push(cur_trim);
            i += 1;
        }
        let joined = para.join("\n");
        let body = render_inline(&joined, options.enable_math, &mut has_math);
        let _ = writeln!(html, "<p data-line=\"{start_line}\">{body}</p>");
    }

    RenderedMarkdown {
        html,
        headings,
        has_math,
        has_mermaid,
    }
}

fn emit_display_math(html: &mut String, tex: &str, enable_math: bool, line: usize) {
    let esc = escape_html(tex);
    if enable_math {
        let _ = writeln!(
            html,
            "<div class=\"math math-display\" data-line=\"{line}\" data-tex=\"{esc}\">$${esc}$$</div>"
        );
    } else {
        let _ = writeln!(html, "<pre data-line=\"{line}\"><code>{esc}</code></pre>");
    }
}

fn render_inline(input: &str, enable_math: bool, has_math: &mut bool) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        // Escaped character
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = input[i + 1..].chars().next().unwrap_or('\\');
            out.push_str(&escape_html(&next.to_string()));
            i += 1 + next.len_utf8();
            continue;
        }

        // Inline code span `` `...` ``
        if bytes[i] == b'`' {
            let tick_len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if let Some(close_rel) = find_closing_ticks(&bytes[i + tick_len..], tick_len) {
                let code = &input[i + tick_len..i + tick_len + close_rel];
                let _ = write!(out, "<code>{}</code>", escape_html(code.trim()));
                i += tick_len + close_rel + tick_len;
                continue;
            }
        }

        // Inline math `$...$` (non-empty, not starting/ending with space, not `$$`)
        if bytes[i] == b'$'
            && bytes.get(i + 1) != Some(&b'$')
            && let Some(end_rel) = find_inline_math_close(&input[i + 1..])
        {
            *has_math = true;
            let tex = &input[i + 1..i + 1 + end_rel];
            let esc = escape_html(tex);
            if enable_math {
                let _ = write!(
                    out,
                    "<span class=\"math math-inline\" data-tex=\"{esc}\">${esc}$</span>"
                );
            } else {
                let _ = write!(out, "<code>{esc}</code>");
            }
            i += 1 + end_rel + 1;
            continue;
        }

        // Image `![alt](url)`
        if bytes[i] == b'!'
            && bytes.get(i + 1) == Some(&b'[')
            && let Some((alt, url, consumed)) = parse_link_or_image(&input[i + 1..])
        {
            let _ = write!(
                out,
                "<img src=\"{}\" alt=\"{}\" />",
                escape_html(url),
                escape_html(alt)
            );
            i += 1 + consumed;
            continue;
        }

        // Link `[text](url)`
        if bytes[i] == b'['
            && let Some((label, url, consumed)) = parse_link_or_image(&input[i..])
        {
            let label_html = render_inline(label, enable_math, has_math);
            let _ = write!(out, "<a href=\"{}\">{label_html}</a>", escape_html(url));
            i += consumed;
            continue;
        }

        // Strikethrough `~~...~~`
        if bytes[i..].starts_with(b"~~")
            && let Some(end_rel) = input[i + 2..].find("~~")
        {
            let inner = render_inline(&input[i + 2..i + 2 + end_rel], enable_math, has_math);
            let _ = write!(out, "<del>{inner}</del>");
            i += 2 + end_rel + 2;
            continue;
        }

        // Strong `**...**`
        if bytes[i..].starts_with(b"**")
            && let Some(end_rel) = input[i + 2..].find("**")
        {
            let inner = render_inline(&input[i + 2..i + 2 + end_rel], enable_math, has_math);
            let _ = write!(out, "<strong>{inner}</strong>");
            i += 2 + end_rel + 2;
            continue;
        }

        // Emphasis `*...*`
        if bytes[i] == b'*'
            && bytes.get(i + 1) != Some(&b'*')
            && bytes.get(i + 1) != Some(&b' ')
            && let Some(end_rel) = input[i + 1..].find('*')
            && end_rel > 0
        {
            let inner = render_inline(&input[i + 1..i + 1 + end_rel], enable_math, has_math);
            let _ = write!(out, "<em>{inner}</em>");
            i += 1 + end_rel + 1;
            continue;
        }

        // GFM autolink `https://...` or `http://...`
        if input[i..].starts_with("https://") || input[i..].starts_with("http://") {
            let end = input[i..]
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | ')'))
                .unwrap_or(input.len() - i);
            let url = input[i..i + end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if url.len() > 8 {
                let esc = escape_html(url);
                let _ = write!(out, "<a href=\"{esc}\">{esc}</a>");
                i += url.len();
                continue;
            }
        }

        // Default: copy next UTF-8 character with HTML escaping (including GFM tagfilter safety).
        let ch = input[i..].chars().next().unwrap_or(' ');
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push(' '),
            other => out.push(other),
        }
        i += ch.len_utf8();
    }
    out
}

fn find_closing_ticks(bytes: &[u8], tick_len: usize) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if run == tick_len {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

fn find_inline_math_close(rest: &str) -> Option<usize> {
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\n') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i > 0 && bytes[i - 1] != b' ' && bytes.get(i + 1) != Some(&b'$') {
                return Some(i);
            }
            return None;
        }
        i += 1;
    }
    None
}

fn parse_link_or_image(rest: &str) -> Option<(&str, &str, usize)> {
    let close_bracket = rest.find(']')?;
    if rest.as_bytes().get(close_bracket + 1) != Some(&b'(') {
        return None;
    }
    let after_paren = &rest[close_bracket + 2..];
    let close_paren = after_paren.find(')')?;
    let label = &rest[1..close_bracket];
    let url = after_paren[..close_paren].trim();
    Some((label, url, close_bracket + 2 + close_paren + 1))
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 3 {
        return false;
    }
    compact.chars().all(|c| c == '-')
        || compact.chars().all(|c| c == '*')
        || compact.chars().all(|c| c == '_')
}

fn parse_ul_item(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
}

fn parse_ol_item(trimmed: &str) -> Option<&str> {
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = &trimmed[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

fn split_table_row(line: &str) -> Vec<&str> {
    let mut s = line.trim();
    if let Some(stripped) = s.strip_prefix('|') {
        s = stripped;
    }
    if let Some(stripped) = s.strip_suffix('|') {
        s = stripped;
    }
    s.split('|').map(str::trim).collect()
}

fn parse_table_delimiter(line: &str) -> Option<Vec<&'static str>> {
    if !line.contains('-') {
        return None;
    }
    let cells = split_table_row(line);
    if cells.is_empty() {
        return None;
    }
    let mut aligns = Vec::with_capacity(cells.len());
    for cell in cells {
        let c = cell.trim();
        if c.is_empty() {
            return None;
        }
        let left = c.starts_with(':');
        let right = c.ends_with(':');
        let inner = c.trim_matches(':');
        if inner.is_empty() || !inner.bytes().all(|b| b == b'-') {
            return None;
        }
        let attr = match (left, right) {
            (true, true) => " style=\"text-align:center\"",
            (false, true) => " style=\"text-align:right\"",
            (true, false) => " style=\"text-align:left\"",
            (false, false) => "",
        };
        aligns.push(attr);
    }
    Some(aligns)
}

/// Escapes special HTML characters.
#[must_use]
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_readme_with_katex_math_mermaid_tables_tasklist_and_github_slugs() {
        let readme = "\
# Physics & Math Guide

Inline energy $E = mc^2$ and display integral:

$$
\\int_0^\\infty e^{-x^2} dx = \\frac{\\sqrt{\\pi}}{2}
$$

## Architecture Diagram

```mermaid
graph TD
    A[Markdown] --> B[KaTeX]
    A --> C[Mermaid]
```

| Feature | Status |
| --- | ---: |
| Math | Ready |

- [x] Heading TOC
- [ ] Sync scroll
";
        let rendered = render_gfm_html(readme, RenderOptions::rich_preview());
        assert!(rendered.has_math);
        assert!(rendered.has_mermaid);
        assert_eq!(rendered.headings.len(), 2);
        assert_eq!(rendered.headings[0].slug, "physics--math-guide");
        assert_eq!(rendered.headings[1].slug, "architecture-diagram");

        assert!(rendered.html.contains("<h1 id=\"physics--math-guide\""));
        assert!(rendered.html.contains("class=\"math math-inline\""));
        assert!(rendered.html.contains("class=\"math math-display\""));
        assert!(rendered.html.contains("<div class=\"mermaid\""));
        assert!(rendered.html.contains("<table>") || rendered.html.contains("<table data-line="));
        assert!(rendered.html.contains("type=\"checkbox\" checked disabled"));
    }
}
