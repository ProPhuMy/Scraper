use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Citizenship, Fact, Source};

static US_CITIZEN_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(us citizens?|us citizenship)\b.*?\b(required|must|only)\b|\b(required|must|only)\b.*?\b(us citizens?|us citizenship)\b")
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

pub fn citizenship(inp: &Input) -> Fact<Citizenship> {
    // 1. Not required first. A sentence with clearance plus (not required | no clearance | not needed) -> NotMentioned, with evidence.
    for sent in &inp.sentences {
        let has_clearance = sent.contains("clearance");
        let not_req = sent.contains("not required") || sent.contains("no clearance") || sent.contains("not needed");
        if has_clearance && not_req {
            return Fact {
                value: Citizenship::NotMentioned,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 2. Required:
    // Form: a question label containing clearance, or citizen (but not authorized to work), or country of citizenship.
    for q in &inp.questions {
        let has_clearance = q.label.contains("clearance");
        let has_citizen = q.label.contains("citizen") && !q.label.contains("authorized to work");
        let has_country = q.label.contains("country of citizenship");
        if has_clearance || has_citizen || has_country {
            return Fact {
                value: Citizenship::Required,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    // Description: any of
    // - us citizen(s|ship) with (required | must | only)
    // - us person
    // - permanent resident with must|only
    // - clearance with (required | must | ability to obtain | able to obtain | eligible to obtain | obtain and maintain | active)
    // - top secret, ts/sci, secret clearance
    for sent in &inp.sentences {
        let citizen_req = US_CITIZEN_REQUIRED_RE.is_match(sent);
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
    // - clearance with preferred|a plus|desired
    // - a bare itar or export control sentence with no us person or citizen
    for sent in &inp.sentences {
        let clearance_pref = sent.contains("clearance") && CLEARANCE_UNKNOWN_RE.is_match(sent);
        let bare_export = (sent.contains("itar") || sent.contains("export control"))
            && !sent.contains("us person")
            && !sent.contains("citizen");

        if clearance_pref || bare_export {
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
            ("Security clearance preferred.", Unknown),
            ("This role is subject to export control regulations.", Unknown),
            ("We build robots.", NotMentioned),
        ];
        for (t, want) in cases {
            assert_eq!(citizenship(&Input::from_text(t)).value, want, "{t}");
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
