use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

pub const LOCAL_EMBEDDING_MODEL: &str = "local-hashed-tfidf-v1";
pub const LOCAL_EMBEDDING_DIM: usize = 256;
pub const DEFAULT_CHUNK_CHARS: usize = 1_400;
pub const DEFAULT_CHUNK_OVERLAP: usize = 200;
const MAX_SECTION_CHARS: usize = 120_000;
const MAX_FALLBACK_BODY_CHARS: usize = 60_000;
const SEMANTIC_WEIGHT: f32 = 0.65;
const LEXICAL_WEIGHT: f32 = 0.35;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractedSection {
    pub section: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractedFiling {
    pub plain_text: String,
    pub sections: Vec<ExtractedSection>,
    pub item_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextChunk {
    pub chunk_index: usize,
    pub section: String,
    pub title: String,
    pub content: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingResult {
    pub model: String,
    pub dim: usize,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone)]
pub enum EmbedderBackend {
    LocalHashedTfIdf,
    OpenAiCompatible {
        base_url: String,
        api_key: String,
        model: String,
    },
}

#[derive(Debug, Clone)]
pub struct Embedder {
    backend: EmbedderBackend,
    client: Option<Client>,
}

impl Embedder {
    pub fn local() -> Self {
        Self {
            backend: EmbedderBackend::LocalHashedTfIdf,
            client: None,
        }
    }

    pub fn from_env() -> Self {
        let api_key = std::env::var("M3_EMBEDDING_API_KEY")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let base_url = std::env::var("M3_EMBEDDING_BASE_URL")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let model = std::env::var("M3_EMBEDDING_MODEL")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        if let (Some(api_key), Some(base_url), Some(model)) = (api_key, base_url, model)
            && let Ok(client) = Client::builder().timeout(Duration::from_secs(20)).build()
        {
            return Self {
                backend: EmbedderBackend::OpenAiCompatible {
                    base_url,
                    api_key,
                    model,
                },
                client: Some(client),
            };
        }
        Self::local()
    }

    #[allow(dead_code)]
    pub fn with_remote(base_url: String, api_key: String, model: String) -> Result<Self> {
        ensure!(!base_url.trim().is_empty(), "embedding base_url is empty");
        ensure!(!api_key.trim().is_empty(), "embedding api_key is empty");
        ensure!(!model.trim().is_empty(), "embedding model is empty");
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("failed to build embedding HTTP client")?;
        Ok(Self {
            backend: EmbedderBackend::OpenAiCompatible {
                base_url: base_url.trim().to_owned(),
                api_key: api_key.trim().to_owned(),
                model: model.trim().to_owned(),
            },
            client: Some(client),
        })
    }

    pub fn model_name(&self) -> &str {
        match &self.backend {
            EmbedderBackend::LocalHashedTfIdf => LOCAL_EMBEDDING_MODEL,
            EmbedderBackend::OpenAiCompatible { model, .. } => model.as_str(),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.backend, EmbedderBackend::OpenAiCompatible { .. })
    }

    pub fn embed(&self, text: &str) -> Result<EmbeddingResult> {
        let mut results = self.embed_batch(&[text])?;
        results
            .pop()
            .context("embedding backend returned empty result")
    }

    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<EmbeddingResult>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        match &self.backend {
            EmbedderBackend::LocalHashedTfIdf => Ok(texts
                .iter()
                .map(|t| {
                    let vec = local_embed(t);
                    EmbeddingResult {
                        model: LOCAL_EMBEDDING_MODEL.to_owned(),
                        dim: vec.len(),
                        vector: vec,
                    }
                })
                .collect()),
            EmbedderBackend::OpenAiCompatible {
                base_url,
                api_key,
                model,
            } => {
                let client = self
                    .client
                    .as_ref()
                    .context("remote embedding HTTP client unavailable")?;
                remote_embed_batch(client, base_url, api_key, model, texts)
            }
        }
    }
}

#[derive(Serialize)]
struct RemoteEmbeddingReq<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct RemoteEmbeddingResp {
    data: Vec<RemoteEmbeddingItem>,
}

#[derive(Deserialize)]
struct RemoteEmbeddingItem {
    #[serde(default)]
    index: usize,
    embedding: Vec<f32>,
}

fn remote_embed_batch(
    client: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    texts: &[&str],
) -> Result<Vec<EmbeddingResult>> {
    let endpoint = format!("{}/embeddings", base_url.trim_end_matches('/'));
    let resp = client
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&RemoteEmbeddingReq {
            model,
            input: texts,
        })
        .send()
        .with_context(|| format!("failed calling embedding endpoint {endpoint}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        let snippet: String = body.chars().take(240).collect();
        anyhow::bail!("embedding endpoint {endpoint} returned HTTP {status}: {snippet}");
    }
    let mut parsed: RemoteEmbeddingResp = resp
        .json()
        .with_context(|| format!("invalid JSON from embedding endpoint {endpoint}"))?;
    ensure!(
        parsed.data.len() == texts.len(),
        "embedding endpoint returned {} vectors for {} inputs",
        parsed.data.len(),
        texts.len()
    );
    parsed.data.sort_by_key(|item| item.index);
    let mut out = Vec::with_capacity(parsed.data.len());
    for item in parsed.data {
        ensure!(
            !item.embedding.is_empty(),
            "embedding endpoint returned empty vector"
        );
        let mut vec = item.embedding;
        l2_normalize(&mut vec);
        let dim = vec.len();
        out.push(EmbeddingResult {
            model: model.to_owned(),
            dim,
            vector: vec,
        });
    }
    Ok(out)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Strips SEC HTML / inline XBRL markup into clean paragraph-preserved plain text.
pub fn strip_html_to_text(raw: &str) -> String {
    let primary = extract_primary_sec_document(raw);
    let mut out = String::with_capacity((primary.len() / 2).min(512 * 1024));
    let bytes = primary.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        let b = bytes[i];
        if b == b'<' {
            let Some(rel_end) = primary[i..].find('>') else {
                // Unclosed '<' at end: emit remaining text literally and stop.
                out.push_str(&primary[i..]);
                break;
            };
            let tag_content = &primary[i + 1..i + rel_end];
            let tag_name = extract_tag_name(tag_content);
            if matches!(
                tag_name.as_str(),
                "script" | "style" | "head" | "noscript" | "ix:header" | "ix:hidden"
            ) && !tag_content.trim_start().starts_with('/')
                && !tag_content.trim_end().ends_with('/')
            {
                let close_needle = format!("</{tag_name}");
                if let Some(close_pos) =
                    find_ascii_case_insensitive(&primary[i + rel_end + 1..], &close_needle)
                {
                    let after_close_start = i + rel_end + 1 + close_pos;
                    if let Some(close_gt) = primary[after_close_start..].find('>') {
                        i = after_close_start + close_gt + 1;
                        continue;
                    }
                }
            }

            if is_paragraph_break_tag(&tag_name) {
                out.push_str("\n\n");
            } else if is_line_break_tag(&tag_name) {
                out.push('\n');
            } else if is_cell_break_tag(&tag_name) {
                out.push(' ');
            }
            i += rel_end + 1;
        } else if b == b'&' {
            if let Some((decoded, consumed)) = decode_html_entity(&primary[i..]) {
                out.push_str(&decoded);
                i += consumed;
            } else {
                out.push('&');
                i += 1;
            }
        } else {
            let ch_len = utf8_char_len(b);
            let end = (i + ch_len).min(len);
            if let Some(slice) = primary.get(i..end) {
                out.push_str(slice);
                i = end;
            } else {
                i += 1;
            }
        }
    }

    normalize_plain_text(&out)
}

fn extract_primary_sec_document(raw: &str) -> &str {
    let Some(doc_start) = find_ascii_case_insensitive(raw, "<DOCUMENT>") else {
        return raw;
    };
    let slice = &raw[doc_start..];
    let text_start = find_ascii_case_insensitive(slice, "<TEXT>").map(|p| p + 6);
    let text_end = find_ascii_case_insensitive(slice, "</TEXT>");
    match (text_start, text_end) {
        (Some(s), Some(e)) if s < e => &slice[s..e],
        (Some(s), None) => &slice[s..],
        _ => raw,
    }
}

fn extract_tag_name(tag_inner: &str) -> String {
    let trimmed = tag_inner.trim_start();
    let stripped = trimmed.strip_prefix('/').unwrap_or(trimmed).trim_start();
    stripped
        .split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_paragraph_break_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "article"
            | "blockquote"
            | "hr"
    )
}

fn is_line_break_tag(tag: &str) -> bool {
    matches!(tag, "br" | "tr" | "li" | "dt" | "dd" | "table")
}

fn is_cell_break_tag(tag: &str) -> bool {
    matches!(tag, "td" | "th" | "span")
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else if first_byte & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return None;
    }
    for pos in 0..=h.len() - n.len() {
        if h[pos..pos + n.len()].eq_ignore_ascii_case(n) {
            return Some(pos);
        }
    }
    None
}

fn decode_html_entity(input: &str) -> Option<(String, usize)> {
    let semi = input[..input.len().min(12)].find(';')?;
    let entity = &input[1..semi];
    let decoded = match entity {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" => "'".to_owned(),
        "nbsp" => " ".to_owned(),
        "ndash" => "-".to_owned(),
        "mdash" => " - ".to_owned(),
        "rsquo" | "lsquo" => "'".to_owned(),
        "rdquo" | "ldquo" => "\"".to_owned(),
        "bull" => "*".to_owned(),
        "hellip" => "...".to_owned(),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            let code = u32::from_str_radix(&entity[2..], 16).ok()?;
            let ch = decode_numeric_char(code)?;
            ch.to_string()
        }
        _ if entity.starts_with('#') => {
            let code = entity[1..].parse::<u32>().ok()?;
            let ch = decode_numeric_char(code)?;
            ch.to_string()
        }
        _ => return None,
    };
    Some((decoded, semi + 1))
}

fn decode_numeric_char(code: u32) -> Option<char> {
    match code {
        160 => Some(' '),
        8211 | 8212 => Some('-'),
        8216 | 8217 => Some('\''),
        8220 | 8221 => Some('"'),
        _ => char::from_u32(code),
    }
}

fn normalize_plain_text(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut blank_run = 0usize;

    for raw_line in raw.lines() {
        let mut collapsed = String::with_capacity(raw_line.len());
        let mut prev_space = false;
        for ch in raw_line.chars() {
            let mapped = if ch == '\u{00a0}' || ch == '\t' {
                ' '
            } else {
                ch
            };
            if mapped == ' ' {
                if !prev_space && !collapsed.is_empty() {
                    collapsed.push(' ');
                    prev_space = true;
                }
            } else if !mapped.is_control() {
                collapsed.push(mapped);
                prev_space = false;
            }
        }
        let trimmed = collapsed.trim().to_owned();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run == 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blank_run = 0;
            lines.push(trimmed);
        }
    }

    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// Extracts structured sections (`10-K` `item_1`/`item_1a`/`item_7`, `8-K` `item_8k` + codes)
/// from raw SEC HTML or plain text.
pub fn extract_sec_sections(form: &str, raw_html_or_text: &str) -> ExtractedFiling {
    let plain_text = strip_html_to_text(raw_html_or_text);
    let upper_form = form.trim().to_ascii_uppercase();
    if upper_form.starts_with("10-K") {
        extract_10k_sections(plain_text)
    } else if upper_form.starts_with("8-K") {
        extract_8k_sections(plain_text)
    } else {
        let item_codes = extract_8k_item_codes(&plain_text);
        let sections = if plain_text.trim().is_empty() {
            Vec::new()
        } else {
            vec![ExtractedSection {
                section: "body".to_owned(),
                title: if upper_form.is_empty() {
                    "Document Body".to_owned()
                } else {
                    format!("{upper_form} Body")
                },
                text: slice_chars(&plain_text, 0, MAX_FALLBACK_BODY_CHARS).to_owned(),
            }]
        };
        ExtractedFiling {
            plain_text,
            sections,
            item_codes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TenKMarker {
    Item1,
    Item1A,
    Item1B,
    Item1C,
    Item2,
    Item7,
    Item7A,
    Item8,
}

#[derive(Debug, Clone)]
struct HeadingMatch {
    marker: TenKMarker,
    byte_offset: usize,
    looks_like_toc: bool,
}

fn extract_10k_sections(plain_text: String) -> ExtractedFiling {
    let matches = scan_10k_headings(&plain_text);
    let targets = [
        (
            TenKMarker::Item1,
            "item_1",
            "Item 1. Business",
            &[
                TenKMarker::Item1A,
                TenKMarker::Item1B,
                TenKMarker::Item1C,
                TenKMarker::Item2,
            ][..],
        ),
        (
            TenKMarker::Item1A,
            "item_1a",
            "Item 1A. Risk Factors",
            &[TenKMarker::Item1B, TenKMarker::Item1C, TenKMarker::Item2][..],
        ),
        (
            TenKMarker::Item7,
            "item_7",
            "Item 7. MD&A",
            &[TenKMarker::Item7A, TenKMarker::Item8][..],
        ),
    ];

    let mut sections = Vec::new();
    for (marker, section_key, title, end_markers) in targets {
        if let Some(body) = pick_best_10k_section(&plain_text, &matches, marker, end_markers) {
            sections.push(ExtractedSection {
                section: section_key.to_owned(),
                title: title.to_owned(),
                text: slice_chars(body, 0, MAX_SECTION_CHARS).to_owned(),
            });
        }
    }

    if sections.is_empty() && !plain_text.trim().is_empty() {
        let fallback_text = clean_fallback_body(&plain_text);
        if !fallback_text.is_empty() {
            sections.push(ExtractedSection {
                section: "body".to_owned(),
                title: "10-K Body".to_owned(),
                text: fallback_text,
            });
        }
    }

    ExtractedFiling {
        plain_text,
        sections,
        item_codes: Vec::new(),
    }
}

fn scan_10k_headings(text: &str) -> Vec<HeadingMatch> {
    let mut matches = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= 160
            && let Some(marker) = detect_10k_heading_marker(trimmed)
        {
            let looks_like_toc = line_looks_like_toc_entry(trimmed);
            matches.push(HeadingMatch {
                marker,
                byte_offset: offset,
                looks_like_toc,
            });
        }
        offset += line.len() + 1;
    }
    matches
}

fn detect_10k_heading_marker(trimmed: &str) -> Option<TenKMarker> {
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("item")?;
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '.');
    if let Some(after) = strip_item_token(rest, "1a") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item1A);
        }
    } else if let Some(after) = strip_item_token(rest, "1b") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item1B);
        }
    } else if let Some(after) = strip_item_token(rest, "1c") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item1C);
        }
    } else if let Some(after) = strip_item_token(rest, "1") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item1);
        }
    } else if let Some(after) = strip_item_token(rest, "2") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item2);
        }
    } else if let Some(after) = strip_item_token(rest, "7a") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item7A);
        }
    } else if let Some(after) = strip_item_token(rest, "7") {
        if after_item_ok(after) {
            return Some(TenKMarker::Item7);
        }
    } else if let Some(after) = strip_item_token(rest, "8")
        && after_item_ok(after)
    {
        return Some(TenKMarker::Item8);
    }
    None
}

fn strip_item_token<'a>(s: &'a str, code: &str) -> Option<&'a str> {
    let after = s.strip_prefix(code)?;
    match after.chars().next() {
        None => Some(after),
        Some(c) if c == '.' || c == ':' || c == '-' || c == '—' || c.is_ascii_whitespace() => {
            Some(after)
        }
        _ => None,
    }
}

fn after_item_ok(after: &str) -> bool {
    let cleaned = after.trim_start_matches(|c: char| {
        c == '.' || c == ':' || c == '-' || c == '—' || c.is_ascii_whitespace()
    });
    if cleaned.is_empty() {
        return true;
    }
    // Reject obvious inline references like "1.1" or "1 of this report".
    !cleaned.starts_with("of ") && !cleaned.starts_with("above") && !cleaned.starts_with("below")
}

fn line_looks_like_toc_entry(trimmed: &str) -> bool {
    if trimmed.contains("....") || trimmed.contains("…") {
        return true;
    }
    let Some(last_tok) = trimmed.split_whitespace().last() else {
        return false;
    };
    let clean_tok = last_tok.trim_matches(|c: char| c == '.' || c == ',');
    !clean_tok.is_empty()
        && clean_tok.len() <= 4
        && clean_tok.chars().all(|c| c.is_ascii_digit())
        && trimmed.split_whitespace().count() >= 3
}

fn pick_best_10k_section<'a>(
    text: &'a str,
    matches: &[HeadingMatch],
    target: TenKMarker,
    end_markers: &[TenKMarker],
) -> Option<&'a str> {
    let mut best_fallback: Option<&'a str> = None;

    for (idx, m) in matches.iter().enumerate() {
        if m.marker != target {
            continue;
        }
        let end_offset = matches[idx + 1..]
            .iter()
            .find(|next| end_markers.contains(&next.marker) && next.byte_offset > m.byte_offset)
            .map(|next| next.byte_offset)
            .unwrap_or(text.len());
        if end_offset <= m.byte_offset {
            continue;
        }
        let slice = text[m.byte_offset..end_offset].trim();
        let char_len = slice.chars().count();

        if !m.looks_like_toc && char_len >= 450 {
            return Some(slice);
        }
        if char_len >= 200
            && best_fallback
                .map(|b| b.chars().count() < char_len)
                .unwrap_or(true)
        {
            best_fallback = Some(slice);
        }
    }

    best_fallback
}

fn clean_fallback_body(plain_text: &str) -> String {
    let total_chars = plain_text.chars().count();
    let start_offset = if total_chars > 4_000 {
        // Skip cover page checkboxes when a filing lacks detectable Item headings.
        let mut bytes = 0usize;
        for (i, ch) in plain_text.char_indices().take(800) {
            bytes = i + ch.len_utf8();
        }
        bytes
    } else {
        0
    };
    slice_chars(&plain_text[start_offset..], 0, MAX_FALLBACK_BODY_CHARS)
        .trim()
        .to_owned()
}

pub fn extract_8k_item_codes(text: &str) -> Vec<String> {
    let mut codes = Vec::new();
    let mut seen = HashSet::new();
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0usize;

    while let Some(rel) = find_ascii_case_insensitive(&lower[i..], "item") {
        let pos = i + rel + 4;
        if pos > 4 && bytes[pos - 5].is_ascii_alphanumeric() {
            i = pos;
            continue;
        }
        let mut j = pos;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'.') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            let code_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j + 2 < bytes.len()
                && bytes[j] == b'.'
                && bytes[j + 1].is_ascii_digit()
                && bytes[j + 2].is_ascii_digit()
                && (j + 3 >= bytes.len() || !bytes[j + 3].is_ascii_digit())
            {
                let code = &lower[code_start..j + 3];
                if seen.insert(code.to_owned()) {
                    codes.push(code.to_owned());
                }
            }
        }
        i = pos;
    }
    codes
}

fn extract_8k_sections(plain_text: String) -> ExtractedFiling {
    let item_codes = extract_8k_item_codes(&plain_text);
    let mut sections = Vec::new();

    if !plain_text.trim().is_empty() {
        let start_byte = find_first_8k_item_offset(&plain_text).unwrap_or(0);
        let tail = &plain_text[start_byte..];
        let end_byte = find_8k_signature_offset(tail).unwrap_or(tail.len());
        let candidate = tail[..end_byte].trim();
        let body = if candidate.chars().count() >= 120 {
            slice_chars(candidate, 0, MAX_SECTION_CHARS).to_owned()
        } else {
            slice_chars(plain_text.trim(), 0, MAX_FALLBACK_BODY_CHARS).to_owned()
        };
        let title = if item_codes.is_empty() {
            "8-K Report".to_owned()
        } else {
            format!("8-K Items {}", item_codes.join(", "))
        };
        sections.push(ExtractedSection {
            section: "item_8k".to_owned(),
            title,
            text: body,
        });
    }

    ExtractedFiling {
        plain_text,
        sections,
        item_codes,
    }
}

fn find_first_8k_item_offset(text: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !extract_8k_item_codes(trimmed).is_empty() {
            return Some(offset);
        }
        offset += line.len() + 1;
    }
    None
}

fn find_8k_signature_offset(text: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in text.split('\n') {
        let upper = line.trim().to_ascii_uppercase();
        if (upper == "SIGNATURE" || upper == "SIGNATURES") && offset >= 200 {
            return Some(offset);
        }
        offset += line.len() + 1;
    }
    None
}

/// Splits text into paragraph-aware overlapping chunks with per-chunk SHA-256 hashes.
pub fn chunk_text(
    section: &str,
    title: &str,
    text: &str,
    max_chars: usize,
    overlap_chars: usize,
) -> Vec<TextChunk> {
    let trimmed = text.trim();
    if trimmed.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let effective_overlap = overlap_chars.min(max_chars / 2);
    let paragraphs = split_paragraphs_bounded(trimmed, max_chars);

    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let sep_len = if current.is_empty() { 0 } else { 2 };
        if current.chars().count() + sep_len + para.chars().count() <= max_chars {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&para);
            continue;
        }

        if !current.trim().is_empty() {
            push_text_chunk(&mut chunks, section, title, current.trim());
            let tail = trailing_word_overlap(current.trim(), effective_overlap);
            current = tail;
        }

        if current.is_empty() {
            current.push_str(&para);
        } else if current.chars().count() + 2 + para.chars().count() <= max_chars {
            current.push_str("\n\n");
            current.push_str(&para);
        } else {
            current.clear();
            current.push_str(&para);
        }
    }

    if !current.trim().is_empty() {
        push_text_chunk(&mut chunks, section, title, current.trim());
    }

    chunks
}

fn push_text_chunk(chunks: &mut Vec<TextChunk>, section: &str, title: &str, content: &str) {
    let sha = sha256_hex(content.as_bytes());
    chunks.push(TextChunk {
        chunk_index: chunks.len(),
        section: section.to_owned(),
        title: title.to_owned(),
        content: content.to_owned(),
        content_sha256: sha,
    });
}

fn split_paragraphs_bounded(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw_para in text.split("\n\n") {
        let p = raw_para.trim();
        if p.is_empty() {
            continue;
        }
        if p.chars().count() <= max_chars {
            out.push(p.to_owned());
            continue;
        }
        // Paragraph is too large: split on single lines, then words/chars cleanly.
        for sub_line in p.split('\n') {
            let sub = sub_line.trim();
            if sub.is_empty() {
                continue;
            }
            if sub.chars().count() <= max_chars {
                out.push(sub.to_owned());
            } else {
                split_long_segment(sub, max_chars, &mut out);
            }
        }
    }
    out
}

fn split_long_segment(segment: &str, max_chars: usize, out: &mut Vec<String>) {
    let mut rest = segment.trim();
    while !rest.is_empty() {
        if rest.chars().count() <= max_chars {
            out.push(rest.to_owned());
            break;
        }
        let hard_byte = char_boundary_after(rest, max_chars);
        let window = &rest[..hard_byte];
        let cut = window
            .rfind(". ")
            .filter(|&pos| pos >= hard_byte / 2)
            .map(|pos| pos + 1)
            .or_else(|| window.rfind(' ').filter(|&pos| pos >= hard_byte / 3))
            .unwrap_or(hard_byte);
        let piece = rest[..cut].trim();
        if !piece.is_empty() {
            out.push(piece.to_owned());
        }
        rest = rest[cut..].trim_start();
    }
}

fn trailing_word_overlap(text: &str, overlap_chars: usize) -> String {
    if overlap_chars == 0 || text.is_empty() {
        return String::new();
    }
    let total_chars = text.chars().count();
    if total_chars <= overlap_chars {
        return text.to_owned();
    }
    let start_byte = char_boundary_after(text, total_chars - overlap_chars);
    let tail = &text[start_byte..];
    if let Some(space_rel) = tail.find(char::is_whitespace)
        && space_rel + 1 < tail.len()
    {
        return tail[space_rel..].trim_start().to_owned();
    }
    tail.trim_start().to_owned()
}

fn char_boundary_after(s: &str, max_chars: usize) -> usize {
    if max_chars == 0 {
        return 0;
    }
    for (count, (byte_idx, ch)) in s.char_indices().enumerate() {
        if count + 1 == max_chars {
            return byte_idx + ch.len_utf8();
        }
    }
    s.len()
}

fn slice_chars(s: &str, start_chars: usize, max_chars: usize) -> &str {
    let start_byte = if start_chars == 0 {
        0
    } else {
        s.char_indices()
            .nth(start_chars)
            .map(|(b, _)| b)
            .unwrap_or(s.len())
    };
    let tail = &s[start_byte..];
    let end_rel = char_boundary_after(tail, max_chars);
    &tail[..end_rel]
}

/// Deterministic zero-cost 256-dim L2-normalized embedding (`local-hashed-tfidf-v1`)
/// using finance unigrams, bigrams, char 4-grams, stopword filtering, sublinear TF,
/// and signed FNV-1a feature hashing.
pub fn local_embed(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0f32; LOCAL_EMBEDDING_DIM];
    let tokens = tokenize_terms(text);
    if tokens.is_empty() {
        return vec;
    }

    let mut feature_counts: HashMap<String, f32> = HashMap::new();
    for tok in &tokens {
        *feature_counts.entry(format!("u:{tok}")).or_insert(0.0) += 1.0;
        if tok.chars().count() >= 4 {
            let chars: Vec<char> = tok.chars().collect();
            for win in chars.windows(4) {
                let g: String = win.iter().collect();
                *feature_counts.entry(format!("c:{g}")).or_insert(0.0) += 1.0;
            }
        }
    }
    for pair in tokens.windows(2) {
        let bg = format!("b:{}_{}", pair[0], pair[1]);
        *feature_counts.entry(bg).or_insert(0.0) += 1.0;
    }

    for (feat, count) in feature_counts {
        let base_weight = if feat.starts_with("b:") {
            1.15f32
        } else if feat.starts_with("c:") {
            0.45f32
        } else {
            1.0f32
        };
        let sublinear_tf = 1.0 + count.ln();
        let h = fnv1a_64(feat.as_bytes());
        let bucket = (h as usize) % LOCAL_EMBEDDING_DIM;
        let sign = if ((h >> 8) & 1) == 0 { 1.0f32 } else { -1.0f32 };
        vec[bucket] += sign * base_weight * sublinear_tf;
    }

    l2_normalize(&mut vec);
    vec
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    // Mix upper bits for cleaner bucket/sign independence.
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

pub fn l2_normalize(vec: &mut [f32]) {
    let norm_sq: f32 = vec.iter().map(|v| v * v).sum();
    if norm_sq > 1e-12 {
        let inv = 1.0 / norm_sq.sqrt();
        for v in vec.iter_mut() {
            *v *= inv;
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = (norm_a * norm_b).sqrt();
    if denom <= 1e-12 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Pre-tokenized query representation for fast repeated BM25 scoring across thousands of chunks.
#[derive(Debug, Clone)]
pub struct PreparedBm25Query {
    weighted_terms: Vec<(String, f32)>,
    total_weight: f32,
}

impl PreparedBm25Query {
    pub fn new(query: &str) -> Self {
        let query_tokens = tokenize_terms(query);
        if query_tokens.is_empty() {
            return Self {
                weighted_terms: Vec::new(),
                total_weight: 0.0,
            };
        }
        let mut q_tf: HashMap<String, f32> = HashMap::new();
        for qt in query_tokens {
            *q_tf.entry(qt).or_insert(0.0) += 1.0;
        }
        let mut weighted_terms: Vec<(String, f32)> = q_tf
            .into_iter()
            .map(|(t, c)| {
                let len_boost = if t.len() >= 6 { 1.15 } else { 1.0 };
                (t, (1.0 + c.ln()) * len_boost)
            })
            .collect();
        weighted_terms.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        weighted_terms.truncate(32);
        let total_weight = weighted_terms.iter().map(|(_, w)| *w).sum();
        Self {
            weighted_terms,
            total_weight,
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.weighted_terms.is_empty()
    }

    pub fn score(&self, doc_text: &str, avg_doc_len: f32) -> f32 {
        if self.weighted_terms.is_empty() || self.total_weight <= 1e-12 {
            return 0.0;
        }
        let doc_tokens = tokenize_terms(doc_text);
        if doc_tokens.is_empty() {
            return 0.0;
        }
        let mut d_tf: HashMap<&str, f32> = HashMap::new();
        for dt in &doc_tokens {
            *d_tf.entry(dt.as_str()).or_insert(0.0) += 1.0;
        }
        let dl = doc_tokens.len() as f32;
        let avg_dl = avg_doc_len.max(20.0);
        let k1 = 1.5f32;
        let b = 0.75f32;
        let dl_norm = (1.0 - b + b * (dl / avg_dl)).max(0.25);

        let mut matched_score = 0.0f32;
        for (term, q_weight) in &self.weighted_terms {
            if let Some(&tf) = d_tf.get(term.as_str()) {
                let bm25_sat = tf / (tf + k1 * dl_norm);
                matched_score += q_weight * bm25_sat;
            }
        }
        (matched_score / self.total_weight).clamp(0.0, 1.0)
    }
}

/// Computes normalized BM25-style lexical overlap score in `[0.0, 1.0]`.
#[allow(dead_code)]
pub fn bm25_overlap_score(query: &str, doc_text: &str, avg_doc_len: f32) -> f32 {
    PreparedBm25Query::new(query).score(doc_text, avg_doc_len)
}

pub fn hybrid_score(semantic: f32, lexical: f32) -> f32 {
    (SEMANTIC_WEIGHT * semantic.clamp(0.0, 1.0) + LEXICAL_WEIGHT * lexical.clamp(0.0, 1.0))
        .clamp(0.0, 1.0)
}

pub fn tokenize_terms(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            push_filtered_token(&cur, &mut tokens);
            cur.clear();
        }
    }
    if !cur.is_empty() {
        push_filtered_token(&cur, &mut tokens);
    }
    tokens
}

fn push_filtered_token(tok: &str, out: &mut Vec<String>) {
    if tok.len() < 2 || tok.len() > 40 {
        return;
    }
    if tok.chars().all(|c| c.is_ascii_digit()) && tok.len() < 4 {
        return;
    }
    if is_stopword(tok) {
        return;
    }
    out.push(tok.to_owned());
}

fn is_stopword(tok: &str) -> bool {
    matches!(
        tok,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "been"
            | "being"
            | "between"
            | "both"
            | "but"
            | "by"
            | "can"
            | "company"
            | "companies"
            | "could"
            | "did"
            | "do"
            | "does"
            | "during"
            | "each"
            | "ended"
            | "fiscal"
            | "for"
            | "form"
            | "from"
            | "had"
            | "has"
            | "have"
            | "having"
            | "he"
            | "her"
            | "here"
            | "his"
            | "how"
            | "if"
            | "in"
            | "including"
            | "into"
            | "is"
            | "it"
            | "item"
            | "its"
            | "may"
            | "might"
            | "more"
            | "most"
            | "must"
            | "no"
            | "not"
            | "of"
            | "on"
            | "or"
            | "other"
            | "our"
            | "out"
            | "over"
            | "page"
            | "per"
            | "period"
            | "periods"
            | "report"
            | "sec"
            | "section"
            | "see"
            | "she"
            | "should"
            | "so"
            | "some"
            | "such"
            | "table"
            | "than"
            | "that"
            | "the"
            | "their"
            | "them"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "through"
            | "to"
            | "under"
            | "up"
            | "us"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "who"
            | "will"
            | "with"
            | "within"
            | "would"
            | "year"
            | "years"
            | "you"
            | "your"
    )
}

pub fn encode_embedding_json(vec: &[f32]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(vec.len() * 9 + 2);
    out.push('[');
    for (idx, &v) in vec.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let clean = if v.is_finite() { v } else { 0.0 };
        let _ = write!(&mut out, "{clean:.6}");
    }
    out.push(']');
    out
}

pub fn decode_embedding_json(raw: &str) -> Option<Vec<f32>> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(LOCAL_EMBEDDING_DIM);
    for part in inner.split(',') {
        let val = part.trim().parse::<f32>().ok()?;
        if !val.is_finite() {
            return None;
        }
        out.push(val);
    }
    Some(out)
}

/// Extracts a verbatim quote from `content` centered on the sentence/line with
/// highest overlap against `query`.
pub fn extract_verbatim_snippet(content: &str, query: &str, max_chars: usize) -> String {
    let clean = content.trim();
    if clean.is_empty() || max_chars == 0 {
        return String::new();
    }
    let q_terms: HashSet<String> = tokenize_terms(query).into_iter().collect();
    if q_terms.is_empty() || clean.chars().count() <= max_chars {
        return truncate_verbatim_chars(clean, max_chars);
    }

    let mut best_window = "";
    let mut best_hits = 0usize;
    for segment in clean.split('\n').flat_map(|l| l.split(". ")) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        let hits = tokenize_terms(seg)
            .into_iter()
            .filter(|t| q_terms.contains(t))
            .collect::<HashSet<_>>()
            .len();
        if hits > best_hits {
            best_hits = hits;
            best_window = seg;
        }
    }

    if best_hits > 0 && !best_window.is_empty() {
        truncate_verbatim_chars(best_window, max_chars)
    } else {
        truncate_verbatim_chars(clean, max_chars)
    }
}

fn truncate_verbatim_chars(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let hard = char_boundary_after(trimmed, max_chars.saturating_sub(3).max(1));
    let window = &trimmed[..hard];
    let cut = window
        .rfind(' ')
        .filter(|&pos| pos >= hard / 2)
        .unwrap_or(hard);
    format!("{}...", trimmed[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_stripper_skips_xbrl_hidden_and_decodes_entities() {
        let html = r#"
        <html>
          <head><title>Ignored</title><style>body{display:none;}</style></head>
          <body>
            <ix:header><ix:hidden>us-gaap:Revenues 123456789</ix:hidden></ix:header>
            <p>Autodesk &amp; Engineering Software &#8212; Cloud &quot;CAD&quot; subscriptions.</p>
            <script>console.log("ignore_me");</script>
            <div>Recurring&nbsp;revenue grew 14%&#39;s year-over-year.</div>
          </body>
        </html>
        "#;
        let text = strip_html_to_text(html);
        assert!(!text.contains("us-gaap:Revenues"));
        assert!(!text.contains("ignore_me"));
        assert!(!text.contains("display:none"));
        assert!(text.contains("Autodesk & Engineering Software"));
        assert!(text.contains("Cloud \"CAD\" subscriptions."));
        assert!(text.contains("Recurring revenue grew 14%'s year-over-year."));
    }

    #[test]
    fn ten_k_extractor_skips_toc_and_finds_items() {
        let mut doc = String::new();
        doc.push_str("<html><body><p>UNITED STATES SECURITIES AND EXCHANGE COMMISSION</p>");
        doc.push_str("<p>Table of Contents</p>");
        doc.push_str("<p>Item 1. Business ................................................ 4</p>");
        doc.push_str("<p>Item 1A. Risk Factors ........................................... 12</p>");
        doc.push_str("<p>Item 7. Management's Discussion and Analysis .................... 30</p>");
        doc.push_str("<p>Item 1. Business</p>");
        doc.push_str("<p>");
        doc.push_str(&"We build industrial design, CAD, and engineering subscription software for architects and manufacturers worldwide. ".repeat(10));
        doc.push_str("</p>");
        doc.push_str("<p>Item 1A. Risk Factors</p>");
        doc.push_str("<p>");
        doc.push_str(&"Macroeconomic downturns and cloud migration execution risks could affect multi-year subscription renewals. ".repeat(10));
        doc.push_str("</p>");
        doc.push_str("<p>Item 2. Properties</p><p>Leased offices.</p>");
        doc.push_str("<p>Item 7. Management's Discussion and Analysis of Financial Condition</p>");
        doc.push_str("<p>");
        doc.push_str(&"Annual recurring revenue and free cash flow expanded due to disciplined operating leverage and low stock-based compensation. ".repeat(10));
        doc.push_str("</p>");
        doc.push_str(
            "<p>Item 8. Financial Statements</p><p>Consolidated statements.</p></body></html>",
        );

        let extracted = extract_sec_sections("10-K", &doc);
        assert_eq!(extracted.sections.len(), 3);
        assert_eq!(extracted.sections[0].section, "item_1");
        assert!(
            extracted.sections[0]
                .text
                .contains("industrial design, CAD, and engineering subscription software")
        );
        assert!(!extracted.sections[0].text.contains(".... 4"));

        assert_eq!(extracted.sections[1].section, "item_1a");
        assert!(
            extracted.sections[1]
                .text
                .contains("cloud migration execution risks")
        );

        assert_eq!(extracted.sections[2].section, "item_7");
        assert!(
            extracted.sections[2]
                .text
                .contains("disciplined operating leverage")
        );
    }

    #[test]
    fn eight_k_extractor_captures_item_codes_and_body() {
        let html = r#"
        <html><body>
          <p>FORM 8-K CURRENT REPORT</p>
          <p>Check the appropriate box below.</p>
          <p>Item 2.02 Results of Operations and Financial Condition.</p>
          <p>On November 10, 2026, Acme Industrial Software Corp. announced third-quarter subscription revenue of $420 million and updated full-year free cash flow guidance.</p>
          <p>Item 5.02 Departure of Directors or Certain Officers.</p>
          <p>The Chief Financial Officer announced his retirement effective next quarter, and the Board appointed an interim successor.</p>
          <p>SIGNATURES</p>
          <p>Pursuant to the requirements...</p>
        </body></html>
        "#;
        let extracted = extract_sec_sections("8-K", html);
        assert_eq!(
            extracted.item_codes,
            vec!["2.02".to_owned(), "5.02".to_owned()]
        );
        assert_eq!(extracted.sections.len(), 1);
        assert_eq!(extracted.sections[0].section, "item_8k");
        assert!(extracted.sections[0].title.contains("2.02"));
        assert!(
            extracted.sections[0]
                .text
                .contains("subscription revenue of $420 million")
        );
        assert!(
            !extracted.sections[0]
                .text
                .contains("Pursuant to the requirements")
        );
    }

    #[test]
    fn chunk_text_respects_overlap_utf8_and_sha256() {
        let para =
            "Enterprise CAD software subscription renewals drive durable cash flow. ".repeat(14);
        let text = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk_text(
            "item_1",
            "Item 1. Business",
            &text,
            DEFAULT_CHUNK_CHARS,
            DEFAULT_CHUNK_OVERLAP,
        );
        assert!(chunks.len() >= 2);
        for (idx, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, idx);
            assert_eq!(chunk.content_sha256.len(), 64);
            assert_eq!(chunk.content_sha256, sha256_hex(chunk.content.as_bytes()));
            assert!(chunk.content.chars().count() <= 1_400);
        }
        // Second chunk should overlap with tail of first chunk.
        let tail_words: Vec<&str> = chunks[0].content.split_whitespace().rev().take(4).collect();
        assert!(chunks[1].content.contains(tail_words[0]));
    }

    #[test]
    fn local_embedder_is_deterministic_normalized_and_semantically_coherent() {
        let embedder = Embedder::local();
        assert!(!embedder.is_remote());
        assert_eq!(embedder.model_name(), LOCAL_EMBEDDING_MODEL);

        let cad_a = embedder
            .embed("Industrial CAD engineering design software recurring subscription model with high gross margin")
            .unwrap();
        let cad_b = embedder
            .embed("Engineering design CAD software subscriptions with recurring cloud maintenance")
            .unwrap();
        let bank = embedder
            .embed(
                "Commercial mortgage deposits interest net interest margin retail banking branches",
            )
            .unwrap();

        assert_eq!(cad_a.dim, LOCAL_EMBEDDING_DIM);
        let norm: f32 = cad_a.vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);

        let sim_close = cosine_similarity(&cad_a.vector, &cad_b.vector);
        let sim_far = cosine_similarity(&cad_a.vector, &bank.vector);
        assert!(
            sim_close > sim_far + 0.15,
            "expected close={sim_close} >> far={sim_far}"
        );

        let encoded = encode_embedding_json(&cad_a.vector);
        let decoded = decode_embedding_json(&encoded).unwrap();
        assert_eq!(decoded.len(), LOCAL_EMBEDDING_DIM);
        assert!((cosine_similarity(&cad_a.vector, &decoded) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hybrid_scoring_and_snippet_extraction_work() {
        let q = "enterprise engineering software subscription";
        let doc_match = "Our enterprise engineering software subscription platform serves manufacturers globally. Gross margin exceeded 85%.";
        let doc_miss =
            "Regional electric utility transmission regulated assets and coal generation.";

        let lex_match = bm25_overlap_score(q, doc_match, 100.0);
        let lex_miss = bm25_overlap_score(q, doc_miss, 100.0);
        assert!(lex_match > 0.4);
        assert_eq!(lex_miss, 0.0);

        let sem_match = cosine_similarity(&local_embed(q), &local_embed(doc_match)).max(0.0);
        let h = hybrid_score(sem_match, lex_match);
        assert!(h > 0.3 && h <= 1.0);

        let quote = extract_verbatim_snippet(doc_match, q, 95);
        assert!(quote.contains("engineering software subscription"));

        let empty_q = PreparedBm25Query::new("the and of");
        assert!(empty_q.is_empty());
        assert_eq!(empty_q.score(doc_match, 100.0), 0.0);

        assert!(Embedder::with_remote("".into(), "k".into(), "m".into()).is_err());
    }
}
