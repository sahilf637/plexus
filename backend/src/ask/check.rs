//! Checking that a model can drive Ask: a made-up two-note library and one easy question, run
//! through the same loop. It says whether the model gave usable steps, whether its answer cited a
//! note correctly, and whether it called tools or had to be asked for JSON.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use serde::Serialize;

use super::budget::Budget;
use super::prompt::Call;
use super::{Model, Run, ToolOutput, Tools, cite};
use crate::db::asks::State;
use crate::error::Error;
use crate::llm::{self, Config, LlmError};
use crate::settings::AskProfile;

const SKY: (&str, &str, &str) = ("check-sky", "Why the sky is blue", "On a clear day the sky looks blue because air scatters blue light much more than red light.");
const SOUP: (&str, &str, &str) = ("check-soup", "Simple soup", "Boil the water, add the vegetables and a pinch of salt, and simmer for twenty minutes.");

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskCheck {
    /// It answered and cited a note correctly.
    pub ok: bool,
    /// `tools` (the model calls tools) or `json` (it is asked for JSON steps).
    pub protocol: &'static str,
    /// Plain words for the person.
    pub message: String,
}

struct Library;

impl Tools for Library {
    fn run(&mut self, call: &Call) -> ToolOutput {
        let card = |n: &str, (_, title, body): (&str, &str, &str)| format!("{n} | {title} | {}", body.chars().take(60).collect::<String>());
        match call {
            Call::Search { .. } => ToolOutput {
                text: format!("{}\n{}", card("n1", SKY), card("n2", SOUP)),
                shown: vec![SKY.0.into(), SOUP.0.into()],
                read: Vec::new(),
                label: None,
            },
            Call::Read { item, .. } if item == "n1" => ToolOutput { text: format!("n1: {}\n\n{}", SKY.1, SKY.2), shown: vec![SKY.0.into()], read: vec![SKY.0.into()], label: None },
            Call::Read { item, .. } if item == "n2" => ToolOutput { text: format!("n2: {}\n\n{}", SOUP.1, SOUP.2), shown: vec![SOUP.0.into()], read: vec![SOUP.0.into()], label: None },
            _ => ToolOutput { text: "Nothing more here.".into(), ..ToolOutput::default() },
        }
    }
}

pub fn check(config: &Config) -> AskCheck {
    judge(config, &|| if llm::tool_calling(config) == Some(true) { "tools" } else { "json" })
}

/// Runs the question through `model` and says how it went; `protocol` tells (once it has run)
/// how the model was called.
fn judge(model: &dyn Model, protocol: &dyn Fn() -> &'static str) -> AskCheck {
    let cards = format!("n1 | {} | {}\nn2 | {} | {}", SKY.1, &SKY.2[..60], SOUP.1, &SOUP.2[..60]);
    let mut run = Run::new("Why is the sky blue?", None, cards, vec![SKY.0.into(), SOUP.0.into()], String::new(), Budget::for_profile(AskProfile::Economy));
    let outcome = run.advance(model, &mut Library, &AtomicBool::new(false), &mut |_| {});
    let protocol = protocol();
    let fail = |message: String| AskCheck { ok: false, protocol, message };

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(Error::Llm(LlmError::Auth(why))) => return fail(format!("The provider didn't accept the key: {why}")),
        Err(Error::Llm(LlmError::RateLimited(_))) => return fail("The provider is limiting requests right now. Try again in a minute.".into()),
        Err(Error::Llm(LlmError::Unavailable(why))) => return fail(format!("Couldn't reach the provider: {why}")),
        Err(error) => return fail(format!("This model couldn't drive a question: {error}. Try another model.")),
    };
    if outcome.state != State::Done {
        return fail("This model ran out of steps on an easy question. Try another model.".into());
    }
    let seen: HashSet<String> = run.shown.union(&run.read).cloned().collect();
    let resolve = |name: &str| match name {
        "n1" => Some(SKY.0.to_string()),
        "n2" => Some(SOUP.0.to_string()),
        _ => None,
    };
    let text_of = |id: &str| match id {
        "check-sky" => Some(format!("{}\n\n{}", SKY.1, SKY.2)),
        "check-soup" => Some(format!("{}\n\n{}", SOUP.1, SOUP.2)),
        _ => None,
    };
    let checked = cite::check(&outcome.text, &outcome.sources, &resolve, &seen, &text_of);
    if checked.sources.is_empty() {
        return fail("This model answered but didn't cite a note correctly, so its answers couldn't be trusted. Try another model.".into());
    }
    let retries = match run.refusals {
        0 => String::new(),
        1 => " One step needed a second try.".to_string(),
        n => format!(" {n} steps needed a second try."),
    };
    let message = if protocol == "tools" {
        format!("Works: this model calls tools and cited the right note.{retries}")
    } else {
        format!("Works, using simpler JSON steps (this model can't call tools). Answers may be a little less reliable.{retries}")
    };
    AskCheck { ok: true, protocol, message }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use serde_json::{Value, json};

    use super::*;
    use crate::llm::Request;

    // `check` takes a provider config, so these test the pieces it is made of.

    struct Script(RefCell<VecDeque<Result<Value, LlmError>>>);

    impl Model for Script {
        fn complete(&self, _request: &Request) -> Result<Value, LlmError> {
            self.0.borrow_mut().pop_front().expect("the script ran out of replies")
        }
    }

    fn run_with(replies: Vec<Result<Value, LlmError>>) -> (Run, Result<super::super::Outcome, Error>) {
        let cards = format!("n1 | {} | {}\nn2 | {} | {}", SKY.1, &SKY.2[..60], SOUP.1, &SOUP.2[..60]);
        let mut run = Run::new("Why is the sky blue?", None, cards, vec![SKY.0.into(), SOUP.0.into()], String::new(), Budget::for_profile(AskProfile::Economy));
        let outcome = run.advance(&Script(RefCell::new(replies.into())), &mut Library, &AtomicBool::new(false), &mut |_| {});
        (run, outcome)
    }

    fn answer(quote: &str) -> Value {
        json!({ "tool": "answer", "text": "Air scatters blue light [1].", "sources": [{ "n": 1, "item": "n1", "quote": quote }] })
    }

    #[test]
    fn the_canned_library_can_be_searched_read_and_cited() {
        let (run, outcome) = run_with(vec![
            Ok(json!({ "tool": "search", "query": "sky" })),
            Ok(json!({ "tool": "read", "item": "n1", "learned": "both notes shown" })),
            Ok(answer("air scatters blue light")),
        ]);
        let outcome = outcome.unwrap();
        assert_eq!(outcome.state, State::Done);
        assert!(run.read.contains(SKY.0));
        let seen: HashSet<String> = run.shown.union(&run.read).cloned().collect();
        let text_of = |_: &str| Some(format!("{}\n\n{}", SKY.1, SKY.2));
        let checked = cite::check(&outcome.text, &outcome.sources, &|n| (n == "n1").then(|| SKY.0.to_string()), &seen, &text_of);
        assert_eq!(checked.sources.len(), 1);
    }

    #[test]
    fn a_made_up_quote_does_not_pass() {
        let (run, outcome) = run_with(vec![Ok(answer("the sky is blue because of the ocean"))]);
        let outcome = outcome.unwrap();
        let seen: HashSet<String> = run.shown.union(&run.read).cloned().collect();
        let text_of = |_: &str| Some(format!("{}\n\n{}", SKY.1, SKY.2));
        let checked = cite::check(&outcome.text, &outcome.sources, &|n| (n == "n1").then(|| SKY.0.to_string()), &seen, &text_of);
        assert!(checked.sources.is_empty() && checked.unsupported);
    }

    fn judged(replies: Vec<Result<Value, LlmError>>, protocol: &'static str) -> AskCheck {
        judge(&Script(RefCell::new(replies.into())), &|| protocol)
    }

    #[test]
    fn a_model_that_answers_and_cites_correctly_passes_in_either_way_of_calling() {
        let good = || vec![Ok(json!({ "tool": "search", "query": "sky" })), Ok(answer("air scatters blue light"))];
        let tools = judged(good(), "tools");
        assert!(tools.ok && tools.protocol == "tools" && tools.message.starts_with("Works: this model calls tools"));
        let json = judged(good(), "json");
        assert!(json.ok && json.message.contains("simpler JSON steps"));
        let retried = judged(vec![Ok(json!({ "tool": "search" })), Ok(answer("air scatters blue light"))], "tools");
        assert!(retried.ok && retried.message.contains("One step needed a second try"));
    }

    #[test]
    fn each_way_a_model_can_fail_is_said_plainly() {
        let made_up = judged(vec![Ok(answer("because of the ocean"))], "json");
        assert!(!made_up.ok && made_up.message.contains("didn't cite a note correctly"));
        let unusable = judged(vec![Ok(json!({})), Ok(json!({}))], "json");
        assert!(!unusable.ok && unusable.message.contains("couldn't drive a question"));
        let key = judged(vec![Err(LlmError::Auth("bad key".into()))], "json");
        assert!(!key.ok && key.message.contains("didn't accept the key"));
        let limited = judged(vec![Err(LlmError::RateLimited(None)); 4], "json");
        assert!(!limited.ok && limited.message.contains("limiting requests"));
        let endless: Vec<_> = (0..12).map(|_| Ok(json!({ "tool": "search", "query": "x" }))).collect();
        let out_of_steps = judged(endless, "json");
        assert!(!out_of_steps.ok && out_of_steps.message.contains("ran out of steps"));
    }

    #[test]
    fn steps_sent_back_are_counted() {
        let (run, outcome) = run_with(vec![Ok(json!({ "tool": "search" })), Ok(answer("air scatters blue light"))]);
        assert!(outcome.is_ok());
        assert_eq!(run.refusals, 1);
    }
}
