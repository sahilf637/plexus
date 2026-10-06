//! What a follow-up carries of the conversation so far: the earlier questions and answers as they
//! were, the sources they cited (title and quote), and the short lines the agent noted as it
//! found things. Not the tool results of earlier turns: those are large, and the notes are still
//! there to be read again. When the conversation is longer than the budget allows, whole old
//! exchanges are left out (their questions stay as a reminder), never cut off in the middle.

use crate::db::asks::{Ask, State};

/// Characters to a token, roughly.
const CHARS_PER_TOKEN: usize = 4;
/// Lines the agent noted that one exchange carries.
const FOUND_LINES: usize = 6;
const FOUND_CHARS: usize = 160;
/// Earlier questions listed when their exchanges were left out.
const REMINDERS: usize = 5;
const REMINDER_CHARS: usize = 120;

pub struct Carried {
    /// To put at the start of the prompt (empty when there is nothing to carry).
    pub text: String,
    /// The notes the carried answers cited, the latest answer's first.
    pub cited: Vec<String>,
    /// The latest question, to help find notes for a short follow-up.
    pub last_question: Option<String>,
}

fn shorten(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    format!("{}…", one_line.chars().take(max).collect::<String>().trim_end())
}

fn exchange(ask: &Ask, title_of: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = format!("Q: {}\nA: {}\n", ask.question.trim(), ask.answer.trim());
    if !ask.sources.is_empty() {
        let sources: Vec<String> = ask
            .sources
            .iter()
            .map(|s| format!("[{}] “{}” — “{}”", s.n, title_of(&s.item_id).unwrap_or_else(|| "a deleted note".into()), shorten(&s.quote, 120)))
            .collect();
        out.push_str(&format!("Sources: {}\n", sources.join("; ")));
    }
    if !ask.found.is_empty() {
        let found: Vec<String> = ask.found.iter().rev().take(FOUND_LINES).rev().map(|l| shorten(l, FOUND_CHARS)).collect();
        out.push_str(&format!("Found along the way: {}\n", found.join("; ")));
    }
    out
}

/// `prior` is the conversation so far, oldest first; `budget_tokens` how much of it to carry.
pub fn carry(prior: &[Ask], title_of: &dyn Fn(&str) -> Option<String>, budget_tokens: usize) -> Carried {
    let usable: Vec<&Ask> = prior
        .iter()
        .filter(|a| matches!(a.state, State::Done | State::Partial | State::Stopped) && !a.answer.trim().is_empty())
        .collect();
    let Some(latest) = usable.last() else { return Carried { text: String::new(), cited: Vec::new(), last_question: None } };
    let limit = budget_tokens * CHARS_PER_TOKEN;

    // Newest first, as many whole exchanges as fit; the newest is always kept.
    let mut kept: Vec<(&Ask, String)> = Vec::new();
    let mut used = 0;
    for ask in usable.iter().rev() {
        let mut text = exchange(ask, title_of);
        if used + text.len() > limit {
            if kept.is_empty() {
                // Even the latest alone is too much: it keeps its question and answer, cut at the limit.
                text = format!("Q: {}\nA: {}\n", ask.question.trim(), shorten(&ask.answer, limit.saturating_sub(ask.question.len() + 12)));
            } else {
                break;
            }
        }
        used += text.len();
        kept.push((ask, text));
    }
    let dropped = &usable[..usable.len() - kept.len()];

    let mut out = String::from("The conversation so far (oldest first):\n");
    if !dropped.is_empty() {
        let reminders: Vec<String> = dropped.iter().rev().take(REMINDERS).rev().map(|a| format!("“{}”", shorten(&a.question, REMINDER_CHARS))).collect();
        out.push_str(&format!("(Earlier exchanges are left out; you were also asked: {}.)\n", reminders.join(", ")));
    }
    for (_, text) in kept.iter().rev() {
        out.push_str(text);
        out.push('\n');
    }
    out.push_str("The numbers in [brackets] above belong to those earlier answers; number the sources of your new answer from [1] again. The question below follows on from this conversation.\n");

    let mut cited: Vec<String> = Vec::new();
    for (ask, _) in &kept {
        for source in &ask.sources {
            if !cited.contains(&source.item_id) {
                cited.push(source.item_id.clone());
            }
        }
    }
    Carried { text: out, cited, last_question: Some(latest.question.clone()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::asks::Source;

    fn ask(n: usize, question: &str, answer: &str, state: State) -> Ask {
        Ask {
            id: format!("a{n}"),
            thread_id: "a1".into(),
            scope: None,
            question: question.into(),
            answer: answer.into(),
            effort: "lookup".into(),
            state,
            unsupported: false,
            limit_hit: None,
            created_at: format!("2026-10-06T09:0{n}:00Z"),
            sources: vec![Source { n: 1, item_id: format!("i{n}"), quote: "one owner per value".into() }],
            found: vec![format!("read Note {n}: something")],
        }
    }

    fn title(id: &str) -> Option<String> {
        Some(format!("Title of {id}"))
    }

    #[test]
    fn a_short_conversation_is_carried_whole_with_sources_and_what_was_found() {
        let prior = vec![ask(1, "What is ownership?", "Each value has one owner [1].", State::Done)];
        let carried = carry(&prior, &title, 4_000);
        assert!(carried.text.contains("Q: What is ownership?\nA: Each value has one owner [1]."));
        assert!(carried.text.contains("[1] “Title of i1” — “one owner per value”"));
        assert!(carried.text.contains("Found along the way: read Note 1: something"));
        assert!(carried.text.contains("number the sources of your new answer from [1] again"));
        assert_eq!(carried.cited, ["i1"]);
        assert_eq!(carried.last_question.as_deref(), Some("What is ownership?"));
    }

    #[test]
    fn nothing_is_carried_from_failed_or_unanswered_questions() {
        let prior = vec![ask(1, "q", "", State::Failed), ask(2, "q", "An answer", State::Failed)];
        let carried = carry(&prior, &title, 4_000);
        assert!(carried.text.is_empty() && carried.cited.is_empty() && carried.last_question.is_none());
    }

    #[test]
    fn a_long_conversation_leaves_out_whole_old_exchanges_and_keeps_their_questions() {
        let long = "word ".repeat(120);
        let prior: Vec<Ask> = (1..=6).map(|n| ask(n, &format!("Question number {n}?"), &long, State::Done)).collect();
        // Room for about two exchanges.
        let carried = carry(&prior, &title, 400);
        assert!(carried.text.contains("Question number 6?") && carried.text.contains("Question number 5?"));
        assert!(!carried.text.contains("Q: Question number 1?"));
        assert!(carried.text.contains("Earlier exchanges are left out"));
        assert!(carried.text.contains("“Question number 1?”"));
        // No exchange is cut off in the middle: every kept answer is whole.
        assert_eq!(carried.text.matches(long.trim()).count(), 2);
        // Oldest first.
        assert!(carried.text.find("Question number 5?").unwrap() < carried.text.find("Question number 6?").unwrap());
        // Only the kept answers' sources are offered as notes to start from.
        assert_eq!(carried.cited, ["i6", "i5"]);
    }

    #[test]
    fn even_one_huge_exchange_is_kept_but_cut_to_the_budget() {
        let huge = "x ".repeat(5_000);
        let carried = carry(&[ask(1, "Big?", &huge, State::Done)], &title, 200);
        assert!(carried.text.contains("Q: Big?"));
        assert!(carried.text.len() < 200 * CHARS_PER_TOKEN + 600);
    }
}
