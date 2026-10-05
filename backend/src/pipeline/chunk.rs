//! Splitting a note into chunks small enough to embed, along markdown boundaries.
//!
//! Adapted from Atomic's chunker (`crates/atomic-core/src/chunking.rs`, MIT, see
//! THIRD_PARTY_NOTICES.md), resized for bge-small's 512-token window:
//! - code blocks stay whole up to the maximum size; bigger ones are split at line boundaries and
//!   re-fenced so every piece still reads as code
//! - headings start new chunks
//! - sizes are estimated from characters (about 4 per token), which is fast and close enough
//! - each chunk ends with a little of the next one, so meaning that spans a boundary isn't lost

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

const TARGET_CHARS: usize = 1200;
const MAX_CHARS: usize = 1600;
const MIN_CHARS: usize = 200;
const OVERLAP_CHARS: usize = 150;

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockType {
    Code,
    Heading,
    List,
    Paragraph,
}

#[derive(Debug)]
struct Block {
    kind: BlockType,
    text: String,
}

pub fn chunk(content: &str) -> Vec<String> {
    let blocks = parse_blocks(content);
    if blocks.is_empty() {
        return Vec::new();
    }
    apply_overlap(merge_small(merge_blocks(blocks)))
}

/// The top-level blocks of the markdown, as text. Paragraphs and code inside lists stay part of
/// the list.
fn parse_blocks(content: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current: Option<(BlockType, usize, usize)> = None;
    let mut flush = |current: &mut Option<(BlockType, usize, usize)>, end: Option<usize>| {
        if let Some((kind, start, last)) = current.take() {
            let text = content[start..end.unwrap_or(last)].trim();
            if !text.is_empty() {
                blocks.push(Block { kind, text: text.to_string() });
            }
        }
    };

    // Lists nest; only the outermost list's end closes the block.
    let mut list_depth = 0usize;
    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => {
                if list_depth == 0 {
                    flush(&mut current, None);
                    current = Some((BlockType::List, range.start, range.end));
                }
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 {
                    flush(&mut current, Some(range.end));
                }
            }
            Event::Start(Tag::CodeBlock(_)) if list_depth == 0 => {
                flush(&mut current, None);
                current = Some((BlockType::Code, range.start, range.end));
            }
            Event::End(TagEnd::CodeBlock) if list_depth == 0 => flush(&mut current, Some(range.end)),
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut current, None);
                current = Some((BlockType::Heading, range.start, range.end));
            }
            Event::End(TagEnd::Heading(_)) => flush(&mut current, Some(range.end)),
            Event::Start(Tag::Paragraph) if current.is_none() => {
                current = Some((BlockType::Paragraph, range.start, range.end));
            }
            Event::End(TagEnd::Paragraph) if matches!(current, Some((BlockType::Paragraph, ..))) => {
                flush(&mut current, Some(range.end));
            }
            // Anything else (text, inline markup, paragraphs inside lists) extends the block.
            _ => {
                if let Some((_, _, end)) = current.as_mut()
                    && range.end > *end
                {
                    *end = range.end;
                }
            }
        }
    }
    flush(&mut current, None);
    blocks
}

/// Packs blocks into chunks of about `TARGET_CHARS`.
fn merge_blocks(blocks: Vec<Block>) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let push = |chunks: &mut Vec<String>, current: &mut String| {
        if !current.is_empty() {
            chunks.push(std::mem::take(current));
        }
    };

    for block in blocks {
        let len = block.text.len();
        match block.kind {
            BlockType::Code => {
                push(&mut chunks, &mut current);
                if len > MAX_CHARS {
                    chunks.extend(split_code(&block.text, MAX_CHARS));
                } else if len > TARGET_CHARS {
                    chunks.push(block.text);
                } else {
                    // A small code block can share a chunk with what follows.
                    current = block.text;
                }
                continue;
            }
            BlockType::Heading if current.len() >= MIN_CHARS => push(&mut chunks, &mut current),
            _ => {}
        }

        if current.len() + len <= TARGET_CHARS {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&block.text);
        } else if len > TARGET_CHARS {
            push(&mut chunks, &mut current);
            for piece in split_sentences(&block.text, TARGET_CHARS) {
                if current.len() + piece.len() > TARGET_CHARS {
                    push(&mut chunks, &mut current);
                }
                current.push_str(&piece);
            }
        } else {
            push(&mut chunks, &mut current);
            current = block.text;
        }
    }
    push(&mut chunks, &mut current);
    chunks
}

/// Folds chunks shorter than `MIN_CHARS` into a neighbour, so tiny fragments don't get their
/// own (noisy) embedding.
fn merge_small(chunks: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for chunk in chunks {
        match result.last_mut() {
            Some(last) if (last.len() < MIN_CHARS || chunk.len() < MIN_CHARS) && last.len() + chunk.len() <= MAX_CHARS => {
                last.push_str("\n\n");
                last.push_str(&chunk);
            }
            _ => result.push(chunk),
        }
    }
    result
}

fn apply_overlap(chunks: Vec<String>) -> Vec<String> {
    let count = chunks.len();
    (0..count)
        .map(|i| match chunks.get(i + 1) {
            Some(next) => format!("{}\n\n{}", chunks[i], prefix(next, OVERLAP_CHARS)),
            None => chunks[i].clone(),
        })
        .collect()
}

/// Splits prose at sentence ends; sentences longer than `max` are cut by length.
fn split_sentences(text: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = [". ", "! ", "? ", ".\n", "!\n", "?\n"]
            .iter()
            .filter_map(|ending| rest.find(ending).map(|pos| pos + ending.len()))
            .min()
            .unwrap_or(rest.len());
        let (sentence, tail) = rest.split_at(end);
        if !current.is_empty() && current.len() + sentence.len() > max {
            pieces.push(std::mem::take(&mut current));
        }
        if sentence.len() > max {
            pieces.extend(hard_split(sentence, max));
        } else {
            current.push_str(sentence);
        }
        rest = tail;
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Cuts text into `max`-byte pieces at character boundaries (last resort).
fn hard_split(text: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            break;
        }
        pieces.push(text[start..end].to_string());
        start = end;
    }
    pieces
}

/// Splits an oversized code block at line boundaries and wraps each piece in the original fence.
fn split_code(block: &str, max: usize) -> Vec<String> {
    let mut lines = block.lines();
    let first = lines.next().unwrap_or("");
    let opener = first.trim_start();
    let fenced = opener.starts_with("```") || opener.starts_with("~~~");
    let (open, close, body): (&str, String, Vec<&str>) = if fenced {
        let fence_char = opener.chars().next().unwrap_or('`');
        let fence_len = opener.chars().take_while(|&c| c == fence_char).count();
        let mut body: Vec<&str> = lines.collect();
        if body.last().is_some_and(|l| l.trim().len() >= 3 && l.trim().chars().all(|c| c == fence_char)) {
            body.pop();
        }
        (first, fence_char.to_string().repeat(fence_len), body)
    } else {
        ("", String::new(), block.lines().collect())
    };
    let budget = max.saturating_sub(open.len() + close.len() + 2).max(64);

    let mut pieces = Vec::new();
    let mut current = String::new();
    for line in body {
        if !current.is_empty() && current.len() + 1 + line.len() > budget {
            pieces.push(std::mem::take(&mut current));
        }
        if line.len() > budget {
            pieces.extend(hard_split(line, budget));
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
        .into_iter()
        .map(|piece| if fenced { format!("{open}\n{piece}\n{close}") } else { piece })
        .collect()
}

fn prefix(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_notes_are_one_chunk() {
        assert!(chunk("").is_empty());
        assert_eq!(chunk("Just a thought."), vec!["Just a thought."]);
        let note = "# Title\n\nFirst paragraph.\n\n- a\n- b\n  - nested\n\nLast.";
        assert_eq!(chunk(note).len(), 1);
        assert!(chunk(note)[0].contains("nested") && chunk(note)[0].contains("Last."));
    }

    #[test]
    fn long_notes_split_within_limits() {
        let paragraph = "Ownership is how Rust manages memory without a garbage collector. ".repeat(12);
        let note = (1..=6).map(|i| format!("## Part {i}\n\n{paragraph}")).collect::<Vec<_>>().join("\n\n");
        let chunks = chunk(&note);
        assert!(chunks.len() >= 4);
        for c in &chunks {
            assert!(c.len() <= MAX_CHARS + OVERLAP_CHARS + 2, "chunk too long: {}", c.len());
        }
        // The text survives chunking: every part heading is somewhere.
        for i in 1..=6 {
            assert!(chunks.iter().any(|c| c.contains(&format!("## Part {i}"))));
        }
    }

    #[test]
    fn code_stays_fenced_when_split() {
        let body = (0..200).map(|i| format!("    let value_{i} = compute({i});")).collect::<Vec<_>>().join("\n");
        let note = format!("```rust\nfn main() {{\n{body}\n}}\n```");
        let chunks = chunk(&note);
        assert!(chunks.len() > 1);
        for c in &chunks {
            let code = c.split("\n\n").next().unwrap();
            assert!(code.starts_with("```rust\n") && code.ends_with("\n```"), "{code}");
        }
    }

    #[test]
    fn one_huge_word_is_cut_safely() {
        let blob = "é".repeat(2000);
        let chunks = chunk(&blob);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().map(|c| c.matches('é').count()).sum::<usize>() >= 2000);
    }
}
