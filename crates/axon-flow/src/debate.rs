//! `debate` — two personas argue, a judge decides (§29.8, §49.2).
//!
//! ```text
//!   for round in 0..rounds:
//!     stance_a = pro.run((question, transcript))
//!     transcript.push((Pro, stance_a))
//!     stance_b = con.run((question, transcript))
//!     transcript.push((Con, stance_b))
//!   verdict = judge.run((question, transcript))
//! ```
//!
//! Each side sees the full transcript so far so positions can sharpen
//! across rounds. The library is opinion-free: rounds, prompts, and judge
//! scoring are entirely the caller's choice; this module only sequences
//! the calls and assembles the transcript.

use serde::{Deserialize, Serialize};

use crate::error::FlowError;
use crate::Step;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Pro,
    Con,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statement {
    pub side: Side,
    pub round: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebateOutcome {
    pub transcript: Vec<Statement>,
    pub verdict: String,
}

/// Run a fixed-rounds debate.
///
/// * `question`        — the prompt both sides argue over.
/// * `pro` / `con`     — step callables: `(question, transcript) -> next_statement`.
/// * `judge`           — final step: `(question, transcript) -> verdict`.
/// * `rounds`          — total argument rounds (0 means judge sees only the question).
pub fn debate<P, C, J>(
    question: String,
    pro: &P,
    con: &C,
    judge: &J,
    rounds: usize,
) -> Result<DebateOutcome, FlowError>
where
    P: Step<(String, Vec<Statement>), String> + ?Sized,
    C: Step<(String, Vec<Statement>), String> + ?Sized,
    J: Step<(String, Vec<Statement>), String> + ?Sized,
{
    let mut transcript: Vec<Statement> = Vec::new();
    for round in 0..rounds {
        let pro_text = pro
            .run((question.clone(), transcript.clone()))
            .map_err(|e| e.with_step(format!("debate[pro:{round}]")))?;
        transcript.push(Statement {
            side: Side::Pro,
            round,
            text: pro_text,
        });
        let con_text = con
            .run((question.clone(), transcript.clone()))
            .map_err(|e| e.with_step(format!("debate[con:{round}]")))?;
        transcript.push(Statement {
            side: Side::Con,
            round,
            text: con_text,
        });
    }
    let verdict = judge
        .run((question, transcript.clone()))
        .map_err(|e| e.with_step("debate[judge]"))?;
    Ok(DebateOutcome {
        transcript,
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scripted_pro(_inp: (String, Vec<Statement>)) -> Result<String, FlowError> {
        Ok("pro-argument".to_string())
    }
    fn scripted_con(inp: (String, Vec<Statement>)) -> Result<String, FlowError> {
        // Slight variance based on transcript length so we can verify
        // ordering downstream.
        Ok(format!("con-argument-after-{}-stmts", inp.1.len()))
    }
    fn scripted_judge(inp: (String, Vec<Statement>)) -> Result<String, FlowError> {
        Ok(format!("verdict-from-{}-statements", inp.1.len()))
    }

    #[test]
    fn runs_pro_then_con_each_round() {
        let pro: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_pro;
        let con: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_con;
        let judge: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_judge;
        let r = debate("Should X?".into(), &pro, &con, &judge, 2).unwrap();
        assert_eq!(r.transcript.len(), 4);
        assert_eq!(r.transcript[0].side, Side::Pro);
        assert_eq!(r.transcript[0].round, 0);
        assert_eq!(r.transcript[1].side, Side::Con);
        assert_eq!(r.transcript[1].round, 0);
        assert_eq!(r.transcript[2].side, Side::Pro);
        assert_eq!(r.transcript[2].round, 1);
        assert_eq!(r.transcript[3].side, Side::Con);
        assert_eq!(r.transcript[3].round, 1);
        assert!(r.verdict.contains("4-statements"));
    }

    #[test]
    fn zero_rounds_judge_only() {
        let pro: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_pro;
        let con: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_con;
        let judge: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_judge;
        let r = debate("Q".into(), &pro, &con, &judge, 0).unwrap();
        assert!(r.transcript.is_empty());
        assert_eq!(r.verdict, "verdict-from-0-statements");
    }

    #[test]
    fn pro_error_short_circuits_with_path() {
        let pro = |_: (String, Vec<Statement>)| {
            Err::<String, _>(FlowError::new("pro down"))
        };
        let con: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_con;
        let judge: fn((String, Vec<Statement>)) -> Result<String, FlowError> = scripted_judge;
        let err = debate("Q".into(), &pro, &con, &judge, 1).unwrap_err();
        assert!(err.path.iter().any(|p| p.contains("pro:0")));
    }
}
