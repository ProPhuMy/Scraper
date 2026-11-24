use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Citizenship, Fact, Source};

static CITIZENSHIP_NEGATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(not required|not needed|not a requirement|is not necessary|not necessary)\b|\bno\s+(\w+\s+){0,3}?(clearance|citizenship)\b")
        .expect("valid regex")
});

static CLEARANCE_NONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bclearance\b[^.!?;\n]{0,40}?(:\s*(none|no|not applicable|n/a)\b|\b(is\s+)?(none|not applicable|n/a)\b)")
        .expect("valid regex")
});

static US_CITIZEN_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(us citizens?|us citizenship)\b.*?\b(required|must)\b|\b(required|must)\b.*?\b(us citizens?|us citizenship)\b|\b(open\s+)?only\s+(to\s+)?(us\s+)?citizens?\b|\b(us\s+)?citizens?\s+only\b")
        .expect("valid regex")
});

static US_PERSON_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bus persons?\b").expect("valid regex")
});

static PERM_RESIDENT_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bpermanent residents?\b.*?\b(must|only)\b|\b(must|only)\b.*?\bpermanent residents?\b")
        .expect("valid regex")
});

static CLEARANCE_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(required|must|ability to obtain|able to obtain|eligible to obtain|obtain and maintain|active)\b")
        .expect("valid regex")
});

static HIGH_CLEARANCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(top secret|ts/sci|secret clearance)\b").expect("valid regex")
});

static CLEARANCE_UNKNOWN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(preferred|a plus|desired)\b").expect("valid regex")
});

static CITIZENSHIP_KEYWORDS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(citizens?|citizenship|clearance|us persons?|itar|export control|permanent residents?|green cards?)\b")
        .expect("valid regex")
});

static HEDGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(may need to|may require|might require|could require|any citizenship)\b")
        .expect("valid regex")
});

static DIRECT_FORM_QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(are you a (us |u\.s\. )?citizen|what is your citizenship status|citizenship status\?|do you (currently )?(hold|have) (an? )?(active )?(us |u\.s\. )?security clearance|current clearance level)\b")
        .expect("valid regex")
});

fn is_citizenship_negated(text: &str) -> bool {
    CITIZENSHIP_NEGATION_RE.is_match(text) || CLEARANCE_NONE_RE.is_match(text)
}

fn is_public_trust_unknown(text: &str) -> bool {
    if !text.contains("public trust") {
        return false;
    }
    !text.contains("citizen")
        && !text.contains("secret")
        && !text.contains("top secret")
        && !text.contains("ts/sci")
        && !text.contains("ts-sci")
}

fn has_non_citizen_alternative(text: &str) -> bool {
    text.contains("or able to obtain an export license")
        || text.contains("export license")
        || text.contains("non-us person")
}

fn is_hedge_unknown(text: &str) -> bool {
    HEDGE_RE.is_match(text)
}

fn is_direct_form_question(text: &str) -> bool {
    DIRECT_FORM_QUESTION_RE.is_match(text)
}

pub fn citizenship(inp: &Input) -> Fact<Citizenship> {
    // 1. Not required first. Clearance or citizenship negated -> NotMentioned, with evidence.
    for sent in &inp.sentences {
        let mentions_topic = sent.contains("clearance") || sent.contains("citizen");
        if mentions_topic && is_citizenship_negated(sent) {
            return Fact {
                value: Citizenship::NotMentioned,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        let mentions_topic = q.label.contains("clearance") || q.label.contains("citizen");
        if mentions_topic && is_citizenship_negated(&q.label) {
            return Fact {
                value: Citizenship::NotMentioned,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }

    // 2. Required:
    // Form: question label
    for q in &inp.questions {
        if is_citizenship_negated(&q.label) {
            continue;
        }
        let is_direct = is_direct_form_question(&q.label);
        if !is_direct {
            if is_public_trust_unknown(&q.label)
                || has_non_citizen_alternative(&q.label)
                || is_hedge_unknown(&q.label)
            {
                continue;
            }
        }

        let has_clearance = q.label.contains("clearance");
        let has_citizen = q.label.contains("citizen") && !q.label.contains("authorized to work");
        let has_country = q.label.contains("country of citizenship");
        let has_us_person = q.label.contains("us person");
        if has_clearance || has_citizen || has_country || has_us_person {
            return Fact {
                value: Citizenship::Required,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    // Description:
    for sent in &inp.sentences {
        if is_citizenship_negated(sent) {
            continue;
        }
        if is_public_trust_unknown(sent) {
            continue;
        }
        if has_non_citizen_alternative(sent) {
            continue;
        }
        if is_hedge_unknown(sent) {
            continue;
        }

        let citizen_req = !sent.contains("not only") && US_CITIZEN_REQUIRED_RE.is_match(sent);
        let us_person = US_PERSON_RE.is_match(sent);
        let perm_res = PERM_RESIDENT_REQUIRED_RE.is_match(sent);
        let clearance_req = sent.contains("clearance") && CLEARANCE_REQUIRED_RE.is_match(sent);
        let high_clearance = HIGH_CLEARANCE_RE.is_match(sent);

        if citizen_req || us_person || perm_res || clearance_req || high_clearance {
            return Fact {
                value: Citizenship::Required,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 3. Unknown:
    for q in &inp.questions {
        if is_public_trust_unknown(&q.label)
            || has_non_citizen_alternative(&q.label)
            || is_hedge_unknown(&q.label)
        {
            return Fact {
                value: Citizenship::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    for sent in &inp.sentences {
        let public_trust = is_public_trust_unknown(sent);
        let alt = has_non_citizen_alternative(sent);
        let hedge = is_hedge_unknown(sent);
        let clearance_pref = sent.contains("clearance") && CLEARANCE_UNKNOWN_RE.is_match(sent);
        let bare_export = (sent.contains("itar") || sent.contains("export control"))
            && !sent.contains("us person")
            && !sent.contains("citizen");

        if public_trust || alt || hedge || clearance_pref || bare_export {
            return Fact {
                value: Citizenship::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 4. Keywords: citizen, clearance, us person, itar, export control, permanent resident, green card.
    for sent in &inp.sentences {
        if CITIZENSHIP_KEYWORDS_RE.is_match(sent) {
            return Fact {
                value: Citizenship::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        if !q.label.contains("authorized to work") && CITIZENSHIP_KEYWORDS_RE.is_match(&q.label) {
            return Fact {
                value: Citizenship::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }

    Fact {
        value: Citizenship::NotMentioned,
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::sponsorship::sponsorship;
    use crate::model::*;

    #[test]
    fn citizenship_table() {
        use Citizenship::*;
        let cases = [
            ("Must be a U.S. citizen.", Required),
            ("Applicants must be able to obtain and maintain a security clearance.", Required),
            ("Active TS/SCI clearance required.", Required),
            ("Due to ITAR, applicants must be U.S. persons.", Required),
            ("No clearance required for this role.", NotMentioned),
            ("No security clearance is required.", NotMentioned),
            ("US citizenship is not required for this role.", NotMentioned),
            ("A clearance is not needed.", NotMentioned),
            ("Citizenship is not a requirement; international students welcome.", NotMentioned),
            ("Security clearance preferred.", Unknown),
            ("This role is subject to export control regulations.", Unknown),
            ("We build robots.", NotMentioned),
            ("minimum clearance required to start: none", NotMentioned),
            ("clearance required for start: no", NotMentioned),
            ("minimum clearance required to start: public trust", Unknown),
            ("- candidates must have the ability to obtain and maintain a public trust clearance", Unknown),
            ("applicants for these positions may need to be \"us persons,\" as defined in these regulations", Unknown),
            ("all applicants are asked to provide documentation that legally establishes status as a us person or non-us person (and nationalities in the case of a non-us person)", Unknown),
            ("- must meet applicable us government contract eligibility requirements, including any citizenship or security clearance requirements associated with the role", Unknown),
            ("- ability to obtain a secret clearance", Required),
            ("due to classified work at the facility and related access restrictions, successful applicants must be us citizens", Required),
            ("- must be a us person due to required access to us export-controlled information or facilities", Required),
            ("clearance required for start: yes", Required),
            ("- active ts/sci clearance", Required),
        ];
        for (t, want) in cases {
            assert_eq!(citizenship(&Input::from_text(t)).value, want, "{t}");
        }
    }

    #[test]
    fn form_question_rules() {
        let cases = [
            ("as a result of the work performed by cesiumastro, all eligible candidates must be a us person or able to obtain an export license. us persons include...", Citizenship::Unknown),
            ("this position may require access to information, technology, software, or hardware that is subject to us export control laws and regulations", Citizenship::Unknown),
            ("do you currently hold an active us security clearance?", Citizenship::Required),
            ("are you a us citizen?", Citizenship::Required),
            ("what is your citizenship status?", Citizenship::Required),
        ];
        for (label, want) in cases {
            let page = PortalPage {
                description: "x".repeat(300),
                questions: vec![FormQuestion {
                    label: label.into(),
                    options: vec![],
                }],
            };
            assert_eq!(citizenship(&Input::new("t", &page)).value, want, "{label}");
        }
    }

    #[test]
    fn eeo_boilerplate_is_ignored() {
        let t = "We consider all applicants without regard to race, color, religion, national origin, citizenship status, or protected veteran status.";
        assert_eq!(citizenship(&Input::from_text(t)).value, Citizenship::NotMentioned);
        assert_eq!(sponsorship(&Input::from_text(t)).value, Sponsorship::NotMentioned);
    }

    #[test]
    fn clearance_form_question_required() {
        let page = PortalPage {
            description: "x".repeat(300),
            questions: vec![FormQuestion {
                label: "Current clearance level".into(),
                options: vec![],
            }],
        };
        assert_eq!(citizenship(&Input::new("t", &page)).value, Citizenship::Required);
    }
}
