//! Allow/deny policy DSL.
//!
//! A `Policy` is an ordered list of `Rule`s; each rule has an action
//! (`Allow` / `Deny`) and a matcher (substring or simple wildcard pattern).
//! Evaluation: first matching rule wins. If no rule matches, the policy's
//! `default` action applies.
//!
//! Wildcard syntax: `*` matches any run of characters, `?` matches one.
//! No regex, no backreferences — small and inspectable.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMatch {
    Contains(String),
    Wildcard(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    pub matcher: RuleMatch,
    /// Optional human-friendly label that propagates into the decision.
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub default: RuleAction,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub action: RuleAction,
    /// Index of the rule that matched, or `None` if the default applied.
    pub rule_index: Option<usize>,
    pub label: String,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            default: RuleAction::Deny,
            rules: Vec::new(),
        }
    }
}

impl Policy {
    pub fn deny_by_default() -> Self {
        Self {
            default: RuleAction::Deny,
            rules: Vec::new(),
        }
    }
    pub fn allow_by_default() -> Self {
        Self {
            default: RuleAction::Allow,
            rules: Vec::new(),
        }
    }

    pub fn rule(mut self, action: RuleAction, matcher: RuleMatch, label: impl Into<String>) -> Self {
        self.rules.push(Rule {
            action,
            matcher,
            label: label.into(),
        });
        self
    }

    pub fn evaluate(&self, input: &str) -> PolicyDecision {
        for (i, rule) in self.rules.iter().enumerate() {
            if rule_matches(&rule.matcher, input) {
                return PolicyDecision {
                    action: rule.action,
                    rule_index: Some(i),
                    label: rule.label.clone(),
                };
            }
        }
        PolicyDecision {
            action: self.default,
            rule_index: None,
            label: format!("default:{:?}", self.default),
        }
    }
}

fn rule_matches(matcher: &RuleMatch, input: &str) -> bool {
    match matcher {
        RuleMatch::Contains(s) => input.contains(s.as_str()),
        RuleMatch::Wildcard(pat) => wildcard_match(pat, input),
    }
}

/// `*` matches any run, `?` matches any single character. Anchored to the
/// full input — wrap with `*...*` if you want substring semantics.
fn wildcard_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = input.chars().collect();
    wildcard_recursive(&p, &s)
}

fn wildcard_recursive(p: &[char], s: &[char]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    match p[0] {
        '*' => {
            // Try matching zero or more characters.
            let rest = &p[1..];
            if wildcard_recursive(rest, s) {
                return true;
            }
            if s.is_empty() {
                return false;
            }
            wildcard_recursive(p, &s[1..])
        }
        '?' => {
            if s.is_empty() {
                return false;
            }
            wildcard_recursive(&p[1..], &s[1..])
        }
        c => {
            if s.is_empty() || s[0] != c {
                return false;
            }
            wildcard_recursive(&p[1..], &s[1..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_deny_with_no_rules_denies() {
        let p = Policy::deny_by_default();
        let d = p.evaluate("anything");
        assert_eq!(d.action, RuleAction::Deny);
        assert_eq!(d.rule_index, None);
    }

    #[test]
    fn allow_rule_overrides_default_deny() {
        let p = Policy::deny_by_default().rule(
            RuleAction::Allow,
            RuleMatch::Contains("greeting:".into()),
            "salutation",
        );
        assert_eq!(p.evaluate("greeting: hello").action, RuleAction::Allow);
        assert_eq!(p.evaluate("payload without it").action, RuleAction::Deny);
    }

    #[test]
    fn first_matching_rule_wins() {
        let p = Policy::allow_by_default()
            .rule(RuleAction::Deny, RuleMatch::Contains("ssn".into()), "pii")
            .rule(RuleAction::Allow, RuleMatch::Contains("ssn".into()), "would-allow");
        let d = p.evaluate("contains ssn here");
        assert_eq!(d.action, RuleAction::Deny);
        assert_eq!(d.rule_index, Some(0));
        assert_eq!(d.label, "pii");
    }

    #[test]
    fn wildcard_matches_anchored_pattern() {
        let p = Policy::deny_by_default().rule(
            RuleAction::Allow,
            RuleMatch::Wildcard("hello *".into()),
            "greeting",
        );
        assert_eq!(p.evaluate("hello world").action, RuleAction::Allow);
        assert_eq!(p.evaluate("say hello world").action, RuleAction::Deny);
    }

    #[test]
    fn json_round_trip_preserves_policy() {
        let p = Policy::allow_by_default()
            .rule(RuleAction::Deny, RuleMatch::Contains("secret".into()), "leak")
            .rule(RuleAction::Allow, RuleMatch::Wildcard("hi*".into()), "greet");
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: Policy = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, p);
    }
}
