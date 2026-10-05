//! Syntax-highlighted lines for JSON values and plain text.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::theme;

/// Pretty JSON, one `Line` per output line, with keys/strings/numbers colored.
pub fn json_lines(v: &Value) -> Vec<Line<'static>> {
    let text = serde_json::to_string_pretty(v).unwrap_or_default();
    text.lines().map(highlight_json_line).collect()
}

/// Colorize one line of pretty-printed JSON. Cheap tokenizer, good enough
/// for display (no multi-line strings in pretty output).
pub fn highlight_json_line(line: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut buf = String::new();
    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>| {
        if !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(buf),
                Style::new().fg(theme::dim()),
            ));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            // string: find the closing quote honoring escapes
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..(i + 1).min(chars.len())].iter().collect();
            i += 1;
            // key if followed by ':'
            let rest: String = chars[i..].iter().collect();
            let is_key = rest.trim_start().starts_with(':');
            flush(&mut buf, &mut spans);
            let color = if is_key {
                theme::key()
            } else {
                theme::string()
            };
            spans.push(Span::styled(s, Style::new().fg(color)));
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '.'
                    || chars[i] == '-'
                    || chars[i] == '+')
            {
                i += 1;
            }
            flush(&mut buf, &mut spans);
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                Style::new().fg(theme::number()),
            ));
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            flush(&mut buf, &mut spans);
            let color = if matches!(word.as_str(), "true" | "false" | "null") {
                theme::keyword()
            } else {
                theme::text()
            };
            spans.push(Span::styled(word, Style::new().fg(color)));
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans);
    Line::from(spans)
}

/// Plain text lines (no highlighting), used for code and logs.
pub fn text_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::new().fg(theme::text()))))
        .collect()
}

/// Lines for either JSON (when it parses) or plain text.
pub fn auto_lines(text: &str) -> Vec<Line<'static>> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) if v.is_object() || v.is_array() => json_lines(&v),
        _ => text_lines(text),
    }
}

/// Compact single-line rendering of a JSON value for table cells.
pub fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn cell_style(v: &Value) -> Style {
    match v {
        Value::Null => Style::new().fg(theme::dim()),
        Value::String(_) => Style::new().fg(theme::text()),
        Value::Number(_) => Style::new().fg(theme::number()),
        Value::Bool(_) => Style::new().fg(theme::keyword()),
        _ => Style::new().fg(theme::string()),
    }
}
