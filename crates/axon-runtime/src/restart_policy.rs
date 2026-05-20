//! Restart policy validation for supervisor children (§29.7).
//!
//! `@restart(Permanent | Transient | Temporary)` annotates a child of
//! a supervisor block. The runtime decides whether to relaunch the
//! child after exit:
//!
//!   * `Permanent` — always restart, no matter how it exited.
//!   * `Transient` — restart only on abnormal exit (panic, error,
//!     non-zero signal); leave alone on a clean return.
//!   * `Temporary` — never restart. One-shot child.
//!
//! The AST stores the attribute argument as a free-form identifier;
//! this module parses + validates it into a typed [`RestartPolicy`]
//! enum so the supervisor doesn't fall through to a silent default
//! when someone typos the label.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Always restart.
    #[default]
    Permanent,
    /// Restart only on abnormal exit.
    Transient,
    /// Never restart.
    Temporary,
}

impl RestartPolicy {
    /// Parse the identifier from `@restart(NAME)`. Accepts the three
    /// PLAN-documented variants; anything else returns
    /// `Err(invalid_name)` so the supervisor's load step can surface a
    /// clean diagnostic.
    pub fn from_attribute_name(name: &str) -> Result<Self, String> {
        match name {
            "Permanent" | "permanent" => Ok(RestartPolicy::Permanent),
            "Transient" | "transient" => Ok(RestartPolicy::Transient),
            "Temporary" | "temporary" => Ok(RestartPolicy::Temporary),
            other => Err(format!(
                "@restart: unknown variant `{other}` (expected Permanent | Transient | Temporary)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RestartPolicy::Permanent => "Permanent",
            RestartPolicy::Transient => "Transient",
            RestartPolicy::Temporary => "Temporary",
        }
    }

    /// Decide whether to restart a child given its exit kind.
    pub fn should_restart(self, exit: ExitKind) -> bool {
        match self {
            RestartPolicy::Permanent => true,
            RestartPolicy::Transient => matches!(exit, ExitKind::Abnormal),
            RestartPolicy::Temporary => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    /// Child returned cleanly from its main loop.
    Normal,
    /// Child panicked, returned an error, or was killed by a signal.
    Abnormal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_three_variants_in_two_cases() {
        for (input, expected) in [
            ("Permanent", RestartPolicy::Permanent),
            ("permanent", RestartPolicy::Permanent),
            ("Transient", RestartPolicy::Transient),
            ("transient", RestartPolicy::Transient),
            ("Temporary", RestartPolicy::Temporary),
            ("temporary", RestartPolicy::Temporary),
        ] {
            assert_eq!(RestartPolicy::from_attribute_name(input).unwrap(), expected);
        }
    }

    #[test]
    fn parse_rejects_unknown_variant_with_helpful_message() {
        let err = RestartPolicy::from_attribute_name("Permanently").unwrap_err();
        assert!(err.contains("Permanent | Transient | Temporary"));
    }

    #[test]
    fn permanent_restarts_on_normal_and_abnormal() {
        assert!(RestartPolicy::Permanent.should_restart(ExitKind::Normal));
        assert!(RestartPolicy::Permanent.should_restart(ExitKind::Abnormal));
    }

    #[test]
    fn transient_restarts_only_on_abnormal() {
        assert!(!RestartPolicy::Transient.should_restart(ExitKind::Normal));
        assert!(RestartPolicy::Transient.should_restart(ExitKind::Abnormal));
    }

    #[test]
    fn temporary_never_restarts() {
        assert!(!RestartPolicy::Temporary.should_restart(ExitKind::Normal));
        assert!(!RestartPolicy::Temporary.should_restart(ExitKind::Abnormal));
    }

    #[test]
    fn name_round_trips_with_parse() {
        for p in [
            RestartPolicy::Permanent,
            RestartPolicy::Transient,
            RestartPolicy::Temporary,
        ] {
            assert_eq!(RestartPolicy::from_attribute_name(p.name()).unwrap(), p);
        }
    }
}
