//! Content filter — flags PII and secret-shaped strings.
//!
//! Pure pattern matching (no regex crate); each detector is hand-rolled
//! against the relevant character classes. False positives are kept low by
//! anchoring on structural shape, not just digit density.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FindingKind {
    Email,
    PhoneUs,
    SsnUs,
    CreditCard,
    ApiKey,
    AwsAccessKey,
    PrivateKeyHeader,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// Byte offset of the match in the input string.
    pub start: usize,
    pub end: usize,
    /// Redacted preview of the match (first few chars, rest as `*`).
    pub redacted: String,
}

#[derive(Clone, Debug, Default)]
pub struct ContentFilter {
    pub scan_email: bool,
    pub scan_phone: bool,
    pub scan_ssn: bool,
    pub scan_credit_card: bool,
    pub scan_api_key: bool,
    pub scan_aws_access_key: bool,
    pub scan_private_key_header: bool,
}

impl ContentFilter {
    /// All detectors on.
    pub fn strict() -> Self {
        Self {
            scan_email: true,
            scan_phone: true,
            scan_ssn: true,
            scan_credit_card: true,
            scan_api_key: true,
            scan_aws_access_key: true,
            scan_private_key_header: true,
        }
    }

    /// Just secret-shaped detectors (skip plain PII like email/phone).
    pub fn secrets_only() -> Self {
        Self {
            scan_api_key: true,
            scan_aws_access_key: true,
            scan_private_key_header: true,
            ..Self::default()
        }
    }

    pub fn scan(&self, text: &str) -> Vec<Finding> {
        let mut out: Vec<Finding> = Vec::new();
        if self.scan_email {
            find_emails(text, &mut out);
        }
        if self.scan_phone {
            find_phones(text, &mut out);
        }
        if self.scan_ssn {
            find_ssns(text, &mut out);
        }
        if self.scan_credit_card {
            find_credit_cards(text, &mut out);
        }
        if self.scan_api_key {
            find_api_keys(text, &mut out);
        }
        if self.scan_aws_access_key {
            find_aws_access_keys(text, &mut out);
        }
        if self.scan_private_key_header {
            find_private_key_headers(text, &mut out);
        }
        out.sort_by_key(|f| (f.start, f.end));
        out
    }
}

// ---- detectors ----------------------------------------------------------

fn push_finding(out: &mut Vec<Finding>, kind: FindingKind, text: &str, start: usize, end: usize) {
    let slice = &text[start..end];
    out.push(Finding {
        kind,
        start,
        end,
        redacted: redact(slice),
    });
}

fn redact(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }
    let keep = (chars.len() / 4).max(2).min(4);
    let mut out: String = chars[..keep].iter().collect();
    out.push_str(&"*".repeat(chars.len() - keep));
    out
}

fn find_emails(text: &str, out: &mut Vec<Finding>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && i > 0 {
            // walk back over the local part
            let mut start = i;
            while start > 0 && is_email_local(bytes[start - 1]) {
                start -= 1;
            }
            if start == i {
                i += 1;
                continue;
            }
            // walk forward over the domain
            let mut end = i + 1;
            while end < bytes.len() && is_email_domain(bytes[end]) {
                end += 1;
            }
            // Domain must contain a dot
            if end > i + 1 && bytes[i + 1..end].contains(&b'.') {
                push_finding(out, FindingKind::Email, text, start, end);
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

fn is_email_local(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}
fn is_email_domain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

fn find_phones(text: &str, out: &mut Vec<Finding>) {
    // Match (NNN) NNN-NNNN or NNN-NNN-NNNN or NNN.NNN.NNNN, optionally
    // preceded by `+1 ` or `1-`. False-positive-prone by nature; we require
    // the structure to keep noise down.
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if let Some(end) = phone_match_at(bytes, i) {
            push_finding(out, FindingKind::PhoneUs, text, i, end);
            i = end;
        } else {
            i += 1;
        }
    }
}

fn phone_match_at(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    // Optional country code "+1 " or "1-".
    if i + 2 < b.len() && b[i] == b'+' && b[i + 1] == b'1' && b[i + 2] == b' ' {
        i += 3;
    } else if i + 1 < b.len() && b[i] == b'1' && b[i + 1] == b'-' {
        i += 2;
    }
    // Optional area-code paren form: "(NNN) "
    let after_area;
    if i < b.len() && b[i] == b'(' {
        if i + 5 > b.len() || !b[i + 1].is_ascii_digit() || !b[i + 2].is_ascii_digit() || !b[i + 3].is_ascii_digit() || b[i + 4] != b')' {
            return None;
        }
        if i + 5 < b.len() && b[i + 5] == b' ' {
            after_area = i + 6;
        } else {
            return None;
        }
    } else {
        if i + 3 >= b.len() {
            return None;
        }
        for k in 0..3 {
            if !b[i + k].is_ascii_digit() {
                return None;
            }
        }
        let sep = b[i + 3];
        if sep != b'-' && sep != b'.' {
            return None;
        }
        after_area = i + 4;
    }
    // NNN[-.]NNNN
    if after_area + 8 > b.len() {
        return None;
    }
    for k in 0..3 {
        if !b[after_area + k].is_ascii_digit() {
            return None;
        }
    }
    if b[after_area + 3] != b'-' && b[after_area + 3] != b'.' {
        return None;
    }
    for k in 0..4 {
        if !b[after_area + 4 + k].is_ascii_digit() {
            return None;
        }
    }
    Some(after_area + 8)
}

fn find_ssns(text: &str, out: &mut Vec<Finding>) {
    // NNN-NN-NNNN with word boundaries.
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 10 < n + 1 {
        if i + 11 <= n
            && bytes[i..i + 3].iter().all(|b| b.is_ascii_digit())
            && bytes[i + 3] == b'-'
            && bytes[i + 4..i + 6].iter().all(|b| b.is_ascii_digit())
            && bytes[i + 6] == b'-'
            && bytes[i + 7..i + 11].iter().all(|b| b.is_ascii_digit())
        {
            let left_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let right_ok = i + 11 == n || !bytes[i + 11].is_ascii_alphanumeric();
            if left_ok && right_ok {
                push_finding(out, FindingKind::SsnUs, text, i, i + 11);
                i += 11;
                continue;
            }
        }
        i += 1;
    }
}

fn find_credit_cards(text: &str, out: &mut Vec<Finding>) {
    // 13-19 digits, possibly separated by single spaces or hyphens between
    // groups, with valid Luhn check. Word boundaries on both sides.
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !bytes[i].is_ascii_digit() || (i > 0 && bytes[i - 1].is_ascii_alphanumeric()) {
            i += 1;
            continue;
        }
        let mut digits: Vec<u8> = Vec::with_capacity(19);
        let mut last_digit_end = i;
        let mut cursor = i;
        while cursor < n && digits.len() < 19 {
            let c = bytes[cursor];
            if c.is_ascii_digit() {
                digits.push(c - b'0');
                cursor += 1;
                last_digit_end = cursor;
            } else if matches!(c, b' ' | b'-')
                && cursor + 1 < n
                && bytes[cursor + 1].is_ascii_digit()
            {
                // Skip one separator iff followed by another digit.
                cursor += 1;
            } else {
                break;
            }
        }
        if digits.len() >= 13 && digits.len() <= 19 && luhn_check(&digits) {
            let right_ok =
                last_digit_end == n || !bytes[last_digit_end].is_ascii_alphanumeric();
            if right_ok {
                push_finding(out, FindingKind::CreditCard, text, i, last_digit_end);
                i = last_digit_end;
                continue;
            }
        }
        i += 1;
    }
}

fn luhn_check(digits: &[u8]) -> bool {
    let mut sum: u32 = 0;
    let n = digits.len();
    for (idx, d) in digits.iter().enumerate() {
        let pos_from_right = n - 1 - idx;
        let v = if pos_from_right % 2 == 1 {
            let doubled = d * 2;
            if doubled > 9 {
                doubled - 9
            } else {
                doubled
            }
        } else {
            *d
        };
        sum += v as u32;
    }
    sum % 10 == 0
}

fn find_api_keys(text: &str, out: &mut Vec<Finding>) {
    // Anthropic / OpenAI / GitHub PAT shapes.
    for prefix in ["sk-ant-", "sk-", "ghp_", "ghu_", "ghr_", "github_pat_"] {
        let mut start = 0;
        while let Some(p) = text[start..].find(prefix) {
            let abs = start + p;
            let mut end = abs + prefix.len();
            while end < text.len() {
                let c = text.as_bytes()[end];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    end += 1;
                } else {
                    break;
                }
            }
            if end - abs >= prefix.len() + 20 {
                push_finding(out, FindingKind::ApiKey, text, abs, end);
            }
            start = end.max(abs + 1);
        }
    }
}

fn find_aws_access_keys(text: &str, out: &mut Vec<Finding>) {
    // AKIA followed by 16 uppercase alnums, or ASIA for STS keys.
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 20 <= n {
        let head = &bytes[i..i + 4];
        if head == b"AKIA" || head == b"ASIA" {
            let mut ok = true;
            for k in 4..20 {
                let c = bytes[i + k];
                if !(c.is_ascii_uppercase() || c.is_ascii_digit()) {
                    ok = false;
                    break;
                }
            }
            let left_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let right_ok = i + 20 == n || !bytes[i + 20].is_ascii_alphanumeric();
            if ok && left_ok && right_ok {
                push_finding(out, FindingKind::AwsAccessKey, text, i, i + 20);
                i += 20;
                continue;
            }
        }
        i += 1;
    }
}

fn find_private_key_headers(text: &str, out: &mut Vec<Finding>) {
    for marker in [
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN EC PRIVATE KEY-----",
        "-----BEGIN DSA PRIVATE KEY-----",
    ] {
        if let Some(p) = text.find(marker) {
            push_finding(out, FindingKind::PrivateKeyHeader, text, p, p + marker.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(findings: &[Finding]) -> Vec<FindingKind> {
        findings.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn detects_email() {
        let f = ContentFilter::strict().scan("contact me at alice@example.com please");
        assert_eq!(kinds(&f), vec![FindingKind::Email]);
    }

    #[test]
    fn does_not_flag_friendly_text_with_no_secrets() {
        let f = ContentFilter::strict()
            .scan("the quick brown fox jumps over 1234 lazy dogs and 5 cats");
        assert!(f.is_empty(), "false positives: {f:?}");
    }

    #[test]
    fn detects_us_phone_in_three_formats() {
        let cf = ContentFilter::strict();
        assert!(!cf.scan("call (555) 867-5309 today").is_empty());
        assert!(!cf.scan("call 555-867-5309 today").is_empty());
        assert!(!cf.scan("call 555.867.5309 today").is_empty());
    }

    #[test]
    fn detects_ssn_with_word_boundaries() {
        let f = ContentFilter::strict().scan("SSN: 123-45-6789 on file");
        assert_eq!(kinds(&f), vec![FindingKind::SsnUs]);
        let nope = ContentFilter::strict().scan("part number 123-45-6789xyz");
        assert!(nope.is_empty(), "right boundary should block");
    }

    #[test]
    fn detects_valid_luhn_credit_card_only() {
        let cf = ContentFilter::strict();
        // 4111 1111 1111 1111 — canonical Visa test number (Luhn valid).
        assert!(!cf.scan("card 4111 1111 1111 1111 here").is_empty());
        // Same digits but bad checksum.
        assert!(cf.scan("card 4111 1111 1111 1112 here").is_empty());
    }

    #[test]
    fn detects_anthropic_and_aws_keys() {
        let cf = ContentFilter::secrets_only();
        let f = cf.scan("key=sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA and AKIAIOSFODNN7EXAMPLE here");
        assert!(kinds(&f).contains(&FindingKind::ApiKey));
        assert!(kinds(&f).contains(&FindingKind::AwsAccessKey));
    }

    #[test]
    fn detects_private_key_header() {
        let f = ContentFilter::secrets_only()
            .scan("oops:\n-----BEGIN RSA PRIVATE KEY-----\nMIIE...");
        assert_eq!(kinds(&f), vec![FindingKind::PrivateKeyHeader]);
    }

    #[test]
    fn redaction_keeps_a_short_prefix() {
        let f = ContentFilter::strict().scan("contact a.b@example.com");
        assert!(f[0].redacted.starts_with('a'));
        assert!(f[0].redacted.contains('*'));
        assert!(!f[0].redacted.contains("example"));
    }
}
