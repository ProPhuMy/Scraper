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

static FORM_CITIZEN_TOPIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(citizens?|citizenship|us persons?|permanent residents?)\b").expect("valid regex")
});

static FORM_REQUIREMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(must|required|requires|require|only|limited to)\b").expect("valid regex")
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
    // Form: question label
    for q in &inp.questions {
        // EEO statements mention "citizenship status" and may end with "acknowledgement required".
        if is_citizenship_negated(&q.label)
            || q.label.contains("equal opportunity")
            || q.label.contains("discriminat")
        {
            continue;
        }
        if is_public_trust_unknown(&q.label)
            || has_non_citizen_alternative(&q.label)
            || is_hedge_unknown(&q.label)
        {
            continue;
        }

        if q.label.contains("clearance") {
            return Fact {
                value: Citizenship::Required,
                evidence: ev(Source::Form, &q.label),
            };
        }

        if FORM_CITIZEN_TOPIC_RE.is_match(&q.label) {
            let cleaned = q.label.replace("information is required", " ");
            if FORM_REQUIREMENT_RE.is_match(&cleaned) {
                return Fact {
                    value: Citizenship::Required,
                    evidence: ev(Source::Form, &q.label),
                };
            }
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
            ("are you a us citizen?", Citizenship::Unknown),
            ("what is your citizenship status?", Citizenship::Unknown),
            ("are you a us citizen, a lawful permanent resident of the united states (\"green card holder\"), an asylee, a refugee, or temporary resident under the 1986 legalization program?", Citizenship::Unknown),
            ("i am one of the following: (a) a citizen of the united states; (b) a lawful permanent resident of the united states; or (c) a person admitted into the united states as an asylee or refugee:", Citizenship::Unknown),
            ("in which country/region do you have citizenship?", Citizenship::Unknown),
            ("export control & compliance: for the sole purpose of determining export licensing requirements, please list any countries of which you are a citizen or legal permanent resident", Citizenship::Unknown),
            ("is your most recent country of (1) citizenship or (2) permanent residency one of the following countries: north korea; syria; iran; or cuba?", Citizenship::Unknown),
            ("are you a us citizen?  note: this information is required by federal, state, or local government contracts.", Citizenship::Unknown),
            ("can you provide the necessary documentation to prove that you are a us citizen, permanent resident, or foreign national with authorization to work in the united states?", Citizenship::Unknown),
            ("you must be a us person because this position requires access to information that is subject to us export controls. are you a us person? (citizen, green card holder, etc.)", Citizenship::Required),
            ("due to federal regulations (e.g., itar/ear), applicants must be us citizens or lawful permanent residents (e.g., current green card holders). are you eligible?", Citizenship::Required),
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
    fn description_citizenship_beats_form_question() {
        let page = PortalPage {
            description: format!("Must be a U.S. citizen. {}", "x".repeat(300)),
            questions: vec![FormQuestion {
                label: "are you a us citizen?".into(),
                options: vec![],
            }],
        };
        assert_eq!(citizenship(&Input::new("t", &page)).value, Citizenship::Required);
    }

    #[test]
    fn eeo_attest_label_not_required() {
        let page = PortalPage {
            description: "x".repeat(300),
            questions: vec![FormQuestion {
                label: "Do you attest to the following statement?

Pareto Captive Services (Pareto\") is an equal opportunity employer. Pareto does not discriminate in employment on account of race, color, religion, national origin, citizenship status, ancestry, age, gender, sexual orientation, marital status, physical or mental disability, military status or unfavorable discharge from military service.

I attest with my signature below that I have given to Pareto true and complete information on this application. Electronic acknowledgement required.".into(),
                options: vec![],
            }],
        };
        assert_ne!(citizenship(&Input::new("t", &page)).value, Citizenship::Required);
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
