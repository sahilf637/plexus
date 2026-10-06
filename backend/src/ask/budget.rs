//! What one question may spend, by the size the person chose (Settings → AI). The same loop runs
//! either way; only the limits change.

use super::prompt::Effort;
use crate::settings::AskProfile;

#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// Tool steps for a lookup, an explanation and a survey.
    steps: [usize; 3],
    /// Prompt tokens a whole question may use, for the same three.
    tokens: [usize; 3],
    /// Longest tool result, in characters.
    pub result_chars: usize,
    /// Notes a search lists.
    pub results: usize,
    /// Notes the first prompt starts from.
    pub candidates: usize,
    /// A page of `read`, and the most one call may ask for.
    pub read_page: usize,
    pub read_max: usize,
    /// How much of the conversation so far a follow-up carries, in tokens.
    pub thread_tokens: usize,
}

impl Budget {
    pub fn for_profile(profile: AskProfile) -> Self {
        match profile {
            AskProfile::Economy => Self {
                steps: [2, 5, 8],
                tokens: [4_000, 15_000, 30_000],
                result_chars: 3_000,
                results: 5,
                candidates: 6,
                read_page: 1_500,
                read_max: 2_000,
                thread_tokens: 1_500,
            },
            AskProfile::Standard => Self {
                steps: [3, 8, 12],
                tokens: [8_000, 30_000, 60_000],
                result_chars: 6_000,
                results: 8,
                candidates: 10,
                read_page: 3_000,
                read_max: 4_000,
                thread_tokens: 4_000,
            },
            AskProfile::Thorough => Self {
                steps: [4, 10, 16],
                tokens: [12_000, 45_000, 90_000],
                result_chars: 9_000,
                results: 12,
                candidates: 14,
                read_page: 4_000,
                read_max: 6_000,
                thread_tokens: 12_000,
            },
        }
    }

    fn index(effort: Effort) -> usize {
        match effort {
            Effort::Lookup => 0,
            Effort::Explain => 1,
            Effort::Survey => 2,
        }
    }

    pub fn steps(&self, effort: Effort) -> usize {
        self.steps[Self::index(effort)]
    }

    pub fn tokens(&self, effort: Effort) -> usize {
        self.tokens[Self::index(effort)]
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::for_profile(AskProfile::Standard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_size_is_roomier_than_the_one_before() {
        let [small, middle, big] = [AskProfile::Economy, AskProfile::Standard, AskProfile::Thorough].map(Budget::for_profile);
        for effort in [Effort::Lookup, Effort::Explain, Effort::Survey] {
            assert!(small.steps(effort) < middle.steps(effort) && middle.steps(effort) < big.steps(effort));
            assert!(small.tokens(effort) < middle.tokens(effort) && middle.tokens(effort) < big.tokens(effort));
        }
        assert!(small.thread_tokens < middle.thread_tokens && middle.thread_tokens < big.thread_tokens);
        assert!(small.result_chars < middle.result_chars && middle.result_chars < big.result_chars);
    }
}
