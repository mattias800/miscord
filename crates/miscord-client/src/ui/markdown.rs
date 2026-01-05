//! Markdown rendering for chat messages
//!
//! Supports:
//! - Headings (# H1, ## H2, ### H3)
//! - Bold (**text** or __text__)
//! - Italic (*text* or _text_)
//! - Inline code (`code`)
//! - Code blocks (```lang\ncode\n```)
//! - Lists (- item or * item)
//!
//! Code blocks have basic syntax highlighting for:
//! - Keywords (blue)
//! - Strings (green)
//! - Comments (gray)
//! - Numbers (orange)

use eframe::egui::{self, Color32, FontId, RichText, Ui};
use regex::Regex;
use std::sync::LazyLock;

// Colors for syntax highlighting (Discord-like dark theme)
const COLOR_KEYWORD: Color32 = Color32::from_rgb(86, 156, 214); // Blue
const COLOR_STRING: Color32 = Color32::from_rgb(152, 195, 121); // Green
const COLOR_COMMENT: Color32 = Color32::from_rgb(106, 115, 125); // Gray
const COLOR_NUMBER: Color32 = Color32::from_rgb(209, 154, 102); // Orange
const COLOR_CODE_BG: Color32 = Color32::from_rgb(40, 42, 54); // Dark background
const COLOR_INLINE_CODE_BG: Color32 = Color32::from_rgb(55, 59, 65); // Slightly lighter
const COLOR_HEADING: Color32 = Color32::from_rgb(220, 220, 220); // Light gray
const COLOR_TEXT: Color32 = Color32::from_rgb(185, 187, 190); // Normal text

// Regex patterns compiled once
static RE_CODE_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"```(\w*)\n([\s\S]*?)```").unwrap());
static RE_INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
static RE_BOLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").unwrap());
// Note: We use a simple pattern here because bold (**) is processed first
// and overlapping matches are filtered out, so we don't need look-around assertions
static RE_ITALIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*([^*]+)\*|_([^_]+)_").unwrap());
static RE_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,3})\s+(.+)$").unwrap());
static RE_LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\-\*]\s+(.+)$").unwrap());

// Syntax highlighting patterns
static RE_COMMENT_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"//.*$").unwrap());
static RE_COMMENT_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/\*[\s\S]*?\*/").unwrap());
static RE_STRING_DOUBLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""[^"]*""#).unwrap());
static RE_STRING_SINGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'[^']*'").unwrap());
static RE_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(fn|let|const|mut|if|else|for|while|loop|return|match|use|pub|struct|enum|impl|self|Self|true|false|None|Some|Ok|Err|null|undefined|var|function|class|import|export|async|await|try|catch|finally|throw|new|this|super|static|private|public|protected|extends|interface|type|where|trait|mod|crate|extern|ref|move|dyn|as|in|of|from|default)\b").unwrap()
});
static RE_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+\.?\d*\b").unwrap());

/// Render markdown text in the UI
pub fn render_markdown(ui: &mut Ui, text: &str) {
    // First, split by code blocks
    let mut last_end = 0;
    let mut parts: Vec<MarkdownPart> = Vec::new();

    for cap in RE_CODE_BLOCK.captures_iter(text) {
        let full_match = cap.get(0).unwrap();

        // Add text before this code block
        if full_match.start() > last_end {
            parts.push(MarkdownPart::Text(text[last_end..full_match.start()].to_string()));
        }

        let lang = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let code = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        parts.push(MarkdownPart::CodeBlock {
            language: lang.to_string(),
            code: code.to_string(),
        });

        last_end = full_match.end();
    }

    // Add remaining text after last code block
    if last_end < text.len() {
        parts.push(MarkdownPart::Text(text[last_end..].to_string()));
    }

    // If no parts, treat entire text as regular text
    if parts.is_empty() {
        parts.push(MarkdownPart::Text(text.to_string()));
    }

    // Render each part
    for part in parts {
        match part {
            MarkdownPart::Text(text) => render_text_block(ui, &text),
            MarkdownPart::CodeBlock { language, code } => render_code_block(ui, &language, &code),
        }
    }
}

#[derive(Debug)]
enum MarkdownPart {
    Text(String),
    CodeBlock { language: String, code: String },
}

/// Render a text block (non-code content)
fn render_text_block(ui: &mut Ui, text: &str) {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }

        // Check for heading
        if let Some(caps) = RE_HEADING.captures(trimmed) {
            let level = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            let content = caps.get(2).map(|m| m.as_str()).unwrap_or(trimmed);

            let font_size = match level {
                1 => 20.0,
                2 => 17.0,
                _ => 15.0,
            };

            ui.label(
                RichText::new(content)
                    .font(FontId::proportional(font_size))
                    .strong()
                    .color(COLOR_HEADING),
            );
            continue;
        }

        // Check for list item
        if let Some(caps) = RE_LIST_ITEM.captures(trimmed) {
            let content = caps.get(1).map(|m| m.as_str()).unwrap_or(trimmed);
            ui.horizontal(|ui| {
                ui.label(RichText::new("  \u{2022}  ").color(COLOR_TEXT)); // Bullet
                render_inline_markdown(ui, content);
            });
            continue;
        }

        // Regular text with inline formatting
        render_inline_markdown(ui, trimmed);
    }
}

/// Render inline markdown (bold, italic, code) on a single line
fn render_inline_markdown(ui: &mut Ui, text: &str) {
    // Build segments with formatting
    let segments = parse_inline_formatting(text);

    if segments.is_empty() {
        ui.label(RichText::new(text).color(COLOR_TEXT));
        return;
    }

    // Use a horizontal layout for inline segments
    ui.horizontal_wrapped(|ui| {
        for segment in segments {
            match segment {
                InlineSegment::Text(s) => {
                    ui.label(RichText::new(s).color(COLOR_TEXT));
                }
                InlineSegment::Bold(s) => {
                    ui.label(RichText::new(s).strong().color(COLOR_TEXT));
                }
                InlineSegment::Italic(s) => {
                    ui.label(RichText::new(s).italics().color(COLOR_TEXT));
                }
                InlineSegment::Code(s) => {
                    ui.label(
                        RichText::new(format!(" {} ", s))
                            .monospace()
                            .background_color(COLOR_INLINE_CODE_BG),
                    );
                }
            }
        }
    });
}

#[derive(Debug)]
enum InlineSegment {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
}

/// Parse inline formatting and return segments
fn parse_inline_formatting(text: &str) -> Vec<InlineSegment> {
    let mut segments = Vec::new();
    let remaining = text.to_string();

    // Process in order: code (highest priority), bold, italic
    // We'll use a simple approach: find all matches and sort by position

    #[derive(Debug)]
    struct Match {
        start: usize,
        end: usize,
        content: String,
        kind: MatchKind,
    }

    #[derive(Debug, Clone, Copy)]
    enum MatchKind {
        Code,
        Bold,
        Italic,
    }

    let mut matches: Vec<Match> = Vec::new();

    // Find inline code
    for cap in RE_INLINE_CODE.captures_iter(&remaining) {
        let full = cap.get(0).unwrap();
        let content = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        matches.push(Match {
            start: full.start(),
            end: full.end(),
            content: content.to_string(),
            kind: MatchKind::Code,
        });
    }

    // Find bold
    for cap in RE_BOLD.captures_iter(&remaining) {
        let full = cap.get(0).unwrap();
        let content = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        matches.push(Match {
            start: full.start(),
            end: full.end(),
            content: content.to_string(),
            kind: MatchKind::Bold,
        });
    }

    // Find italic
    for cap in RE_ITALIC.captures_iter(&remaining) {
        let full = cap.get(0).unwrap();
        let content = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        matches.push(Match {
            start: full.start(),
            end: full.end(),
            content: content.to_string(),
            kind: MatchKind::Italic,
        });
    }

    // Sort by start position
    matches.sort_by_key(|m| m.start);

    // Remove overlapping matches (keep first one)
    let mut filtered: Vec<Match> = Vec::new();
    let mut last_end = 0;
    for m in matches {
        if m.start >= last_end {
            last_end = m.end;
            filtered.push(m);
        }
    }

    // Build segments
    let mut pos = 0;
    for m in filtered {
        if m.start > pos {
            segments.push(InlineSegment::Text(remaining[pos..m.start].to_string()));
        }
        match m.kind {
            MatchKind::Code => segments.push(InlineSegment::Code(m.content)),
            MatchKind::Bold => segments.push(InlineSegment::Bold(m.content)),
            MatchKind::Italic => segments.push(InlineSegment::Italic(m.content)),
        }
        pos = m.end;
    }

    // Add remaining text
    if pos < remaining.len() {
        segments.push(InlineSegment::Text(remaining[pos..].to_string()));
    }

    segments
}

/// Render a code block with syntax highlighting
fn render_code_block(ui: &mut Ui, _language: &str, code: &str) {
    egui::Frame::none()
        .fill(COLOR_CODE_BG)
        .rounding(4.0)
        .inner_margin(8.0)
        .outer_margin(egui::Margin::symmetric(0.0, 4.0))
        .show(ui, |ui| {
            // Apply syntax highlighting
            let highlighted = highlight_syntax(code);

            for line in highlighted.lines() {
                render_highlighted_line(ui, line);
            }
        });
}

/// Apply syntax highlighting to code
fn highlight_syntax(code: &str) -> String {
    // We'll mark regions with special markers that we'll parse when rendering
    // Format: \x01TYPE\x02content\x03
    // Where TYPE is: K=keyword, S=string, C=comment, N=number

    let mut result = code.to_string();

    // Mark comments first (so we don't highlight inside them)
    result = mark_matches(&result, &RE_COMMENT_BLOCK, 'C');
    result = mark_matches(&result, &RE_COMMENT_LINE, 'C');

    // Mark strings (so we don't highlight inside them)
    result = mark_matches(&result, &RE_STRING_DOUBLE, 'S');
    result = mark_matches(&result, &RE_STRING_SINGLE, 'S');

    // Mark keywords (only outside comments and strings)
    result = mark_matches_safe(&result, &RE_KEYWORD, 'K');

    // Mark numbers (only outside comments and strings)
    result = mark_matches_safe(&result, &RE_NUMBER, 'N');

    result
}

/// Mark regex matches with type markers
fn mark_matches(text: &str, re: &Regex, mark_type: char) -> String {
    re.replace_all(text, |caps: &regex::Captures| {
        format!("\x01{}\x02{}\x03", mark_type, &caps[0])
    })
    .to_string()
}

/// Mark matches only if they're not inside existing markers
fn mark_matches_safe(text: &str, re: &Regex, mark_type: char) -> String {
    let mut result = String::new();
    let mut in_marker = false;

    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == 0x01 {
            in_marker = true;
            result.push(bytes[i] as char);
            i += 1;
            continue;
        }
        if bytes[i] == 0x03 {
            in_marker = false;
            result.push(bytes[i] as char);
            i += 1;
            continue;
        }

        if in_marker {
            result.push(bytes[i] as char);
            i += 1;
        } else {
            // Check if regex matches at this position
            if let Some(m) = re.find(&text[i..]) {
                if m.start() == 0 {
                    // Match at current position
                    result.push('\x01');
                    result.push(mark_type);
                    result.push('\x02');
                    result.push_str(m.as_str());
                    result.push('\x03');
                    i += m.len();
                    continue;
                }
            }
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Render a single line with syntax highlighting
fn render_highlighted_line(ui: &mut Ui, line: &str) {
    // Parse markers and render with colors
    let segments = parse_highlighted_segments(line);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0; // No spacing between segments

        for (text, color) in segments {
            ui.label(RichText::new(text).monospace().color(color));
        }
    });
}

/// Parse highlighted segments from marked text
fn parse_highlighted_segments(text: &str) -> Vec<(String, Color32)> {
    let mut segments = Vec::new();
    let mut current_text = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x01' {
            // Start of marked region
            if !current_text.is_empty() {
                segments.push((current_text.clone(), COLOR_TEXT));
                current_text.clear();
            }

            // Get type
            let type_char = chars.next().unwrap_or('T');
            // Skip \x02
            chars.next();

            // Collect until \x03
            let mut marked_text = String::new();
            while let Some(c) = chars.next() {
                if c == '\x03' {
                    break;
                }
                marked_text.push(c);
            }

            let color = match type_char {
                'K' => COLOR_KEYWORD,
                'S' => COLOR_STRING,
                'C' => COLOR_COMMENT,
                'N' => COLOR_NUMBER,
                _ => COLOR_TEXT,
            };

            segments.push((marked_text, color));
        } else {
            current_text.push(c);
        }
    }

    if !current_text.is_empty() {
        segments.push((current_text, COLOR_TEXT));
    }

    segments
}
