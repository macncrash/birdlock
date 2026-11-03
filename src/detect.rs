//! Existence checks.
//!
//! WhatsMyName sites carry an "exists" code and string plus a "miss" code and
//! string. Sherlock sites carry one or more of message, status_code, and
//! response_url. When both lists describe the same probe, every rule has to
//! agree before the account is reported as found.

use crate::catalog::{Rule, SherlockKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Found,
    Missing,
    Disputed,
    Waf,
    Unknown,
}

/// Challenge pages that Sherlock treats as a block rather than an account.
const WAF_MARKS: &[&str] = &[
    ".loading-spinner{visibility:hidden}body.no-js .challenge-running{display:none}body.dark{background-color:#222;color:#d9d9d9}body.dark a{color:#fff}body.dark a:hover{color:#ee730a;text-decoration:underline}body.dark .lds-ring div{border-color:#999 transparent transparent}body.dark .font-red{color:#b20f03}body.dark",
    "<span id=\"challenge-error-text\">",
    "AwsWafIntegration.forceRefreshToken",
    "{return l.onPageView}}),Object.defineProperty(r,\"perimeterxIdentifiers\",{enumerable:",
];

pub fn is_waf(body: &str) -> bool {
    WAF_MARKS.iter().any(|mark| body.contains(mark))
}

pub fn judge(rules: &[Rule], status: i64, body: &str) -> Verdict {
    if is_waf(body) {
        return Verdict::Waf;
    }
    if rules.is_empty() {
        return Verdict::Unknown;
    }
    let parts: Vec<Verdict> = rules
        .iter()
        .map(|rule| judge_one(rule, status, body))
        .collect();
    combine(&parts)
}

fn combine(parts: &[Verdict]) -> Verdict {
    if parts.iter().any(|p| *p == Verdict::Waf) {
        return Verdict::Waf;
    }
    let found = parts.iter().any(|p| *p == Verdict::Found);
    let missing = parts.iter().any(|p| *p == Verdict::Missing);
    if parts.iter().all(|p| *p == Verdict::Found) {
        Verdict::Found
    } else if found && missing {
        Verdict::Disputed
    } else if missing {
        Verdict::Missing
    } else {
        Verdict::Unknown
    }
}

fn judge_one(rule: &Rule, status: i64, body: &str) -> Verdict {
    match rule {
        Rule::Wmn {
            e_code,
            e_string,
            m_code,
            m_string,
        } => wmn(status, body, *e_code, e_string, *m_code, m_string),
        Rule::Sherlock {
            kinds,
            messages,
            codes,
        } => sherlock(status, body, kinds, messages, codes),
    }
}

/// Empty exists-string means "any body", matching Python `"" in text`.
/// Empty miss-string means "no miss marker". Blackbird treats that as a miss
/// because `"" not in text` is always false, so those sites never hit.
fn wmn(
    status: i64,
    body: &str,
    e_code: i64,
    e_string: &str,
    m_code: i64,
    m_string: &str,
) -> Verdict {
    let exists = status == e_code && (e_string.is_empty() || body.contains(e_string));
    if !exists {
        return Verdict::Missing;
    }
    if !m_string.is_empty() && body.contains(m_string) {
        return Verdict::Missing;
    }
    if m_code != e_code && status == m_code {
        return Verdict::Missing;
    }
    Verdict::Found
}

fn sherlock(
    status: i64,
    body: &str,
    kinds: &[SherlockKind],
    messages: &[String],
    codes: &[i64],
) -> Verdict {
    let mut current = Verdict::Unknown;
    if kinds.contains(&SherlockKind::Message) {
        let saw_error = messages
            .iter()
            .any(|msg| !msg.is_empty() && body.contains(msg));
        current = if saw_error {
            Verdict::Missing
        } else {
            Verdict::Found
        };
    }
    if kinds.contains(&SherlockKind::StatusCode) && current != Verdict::Missing {
        let mut claimed = (200..300).contains(&status);
        if !codes.is_empty() && codes.contains(&status) {
            claimed = false;
        }
        current = if claimed {
            Verdict::Found
        } else {
            Verdict::Missing
        };
    }
    if kinds.contains(&SherlockKind::ResponseUrl) && current != Verdict::Missing {
        current = if (200..300).contains(&status) {
            Verdict::Found
        } else {
            Verdict::Missing
        };
    }
    current
}

/// 3xx exists/miss codes and Sherlock's response_url check need the status
/// from the first response, before a redirect is followed.
pub fn wants_raw_status(rules: &[Rule]) -> bool {
    for rule in rules {
        match rule {
            Rule::Wmn { e_code, m_code, .. } => {
                if (300..400).contains(e_code) || (300..400).contains(m_code) {
                    return true;
                }
            }
            Rule::Sherlock { kinds, .. } => {
                if kinds.contains(&SherlockKind::ResponseUrl) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wmn_rule(e_code: i64, e: &str, m_code: i64, m: &str) -> Rule {
        Rule::Wmn {
            e_code,
            e_string: e.into(),
            m_code,
            m_string: m.into(),
        }
    }

    #[test]
    fn wmn_same_code_and_empty_miss_string() {
        let same = wmn_rule(200, "staff", 200, "missing-page");
        assert_eq!(judge(&[same.clone()], 200, "staff here"), Verdict::Found);
        assert_eq!(judge(&[same], 200, "staff missing-page"), Verdict::Missing);

        let empty_miss = wmn_rule(200, "userNameBlock", 302, "");
        assert_eq!(judge(&[empty_miss], 200, "userNameBlock"), Verdict::Found);

        let redirect = wmn_rule(302, "", 404, "nope");
        assert_eq!(judge(&[redirect], 302, "anything"), Verdict::Found);
        assert!(wants_raw_status(&[wmn_rule(302, "", 404, "x")]));
    }

    #[test]
    fn sherlock_rules_and_waf() {
        let msg = Rule::Sherlock {
            kinds: vec![SherlockKind::Message],
            messages: vec!["Not Found".into()],
            codes: vec![],
        };
        assert_eq!(judge(&[msg.clone()], 200, "hello"), Verdict::Found);
        assert_eq!(judge(&[msg], 200, "Not Found"), Verdict::Missing);

        let code = Rule::Sherlock {
            kinds: vec![SherlockKind::StatusCode],
            messages: vec![],
            codes: vec![404],
        };
        assert_eq!(judge(&[code.clone()], 200, ""), Verdict::Found);
        assert_eq!(judge(&[code.clone()], 404, ""), Verdict::Missing);
        assert_eq!(judge(&[code], 500, ""), Verdict::Missing);

        let both = vec![
            wmn_rule(200, "profile", 404, "gone"),
            Rule::Sherlock {
                kinds: vec![SherlockKind::StatusCode],
                messages: vec![],
                codes: vec![],
            },
        ];
        assert_eq!(judge(&both, 200, "profile"), Verdict::Found);
        assert_eq!(judge(&both, 200, "nope"), Verdict::Disputed);
        assert_eq!(
            judge(&both, 200, "<span id=\"challenge-error-text\">"),
            Verdict::Waf
        );
    }
}
