use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Fact, Source, Sponsorship};

static CPT_OPT_WORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(cpt|opt|curricular practical training|optional practical training)\b")
        .expect("valid regex")
});

static CPT_OPT_REFUSED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(ineligible|unacceptable)\b|\b(not|never|cannot|can't|won't|unable to)\s+(\w+\s+){0,3}?(accept\w*|eligible|consider\w*|support\w*)\b")
        .expect("valid regex")
});

static CITIZENS_ONLY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(open\s+)?only\s+(to\s+)?(us\s+)?citizens?\b|\b(us\s+)?citizens?\s+only\b")
        .expect("valid regex")
});

static MUST_BE_CITIZEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bmust be\b.*?\b(us citizens?|permanent residents?|green cards?|us persons?)\b")
        .expect("valid regex")
});

static SPONSOR_FUTURE_WITHOUT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(now or in the future|currently or in the future|now or at any time in the future)\b")
        .expect("valid regex")
});

static CPT_OPT_OFFERED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(welcome\w*|accepted|eligible|encouraged|supported)\b")
        .expect("valid regex")
});

static CLAUSE_SPLIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r",\s*but\s+|;\s*however\s+|\s+but\s+|;\s*")
        .expect("valid regex")
});

static SPONSOR_NEGATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(not|unable|cannot|can't|won't|no)\b|\b(not available|unavailable|not offered|not provided)\b")
        .expect("valid regex")
});

static SPONSORSHIP_AVAILABLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(available|offered|provided|is possible)\b")
        .expect("valid regex")
});

static KEYWORD_SPONSOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bsponsor\w*\b|\bh-?1b\b|\bcpt\b|\bopt\b|\bf-?1\b")
        .expect("valid regex")
});

static VISA_RELATED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(sponsor\w*|status|students?|work)\b")
        .expect("valid regex")
});

fn is_cpt_opt_offered_clause(clause: &str) -> bool {
    let has_cpt_opt = CPT_OPT_WORD_RE.is_match(clause);
    let has_offered = CPT_OPT_OFFERED_RE.is_match(clause);
    let has_negation = clause.contains("not")
        || clause.contains("no ")
        || clause.contains("unable")
        || clause.contains("cannot")
        || clause.contains("can't")
        || clause.contains("won't");
    has_cpt_opt && has_offered && !has_negation
}

fn has_cpt_opt_offered(sent: &str) -> bool {
    CLAUSE_SPLIT_RE.split(sent).any(is_cpt_opt_offered_clause)
}

pub fn sponsorship(inp: &Input) -> Fact<Sponsorship> {
    // 1. NotOffered. Description sentences only, never form labels.
    for sent in &inp.sentences {
        let cpt_refused = CPT_OPT_WORD_RE.is_match(sent)
            && CPT_OPT_REFUSED_RE.is_match(sent)
            && !has_cpt_opt_offered(sent);
        let citizens_only = !sent.contains("not only")
            && !crate::extract::citizenship::has_non_citizen_alternative(sent)
            && (CITIZENS_ONLY_RE.is_match(sent) || MUST_BE_CITIZEN_RE.is_match(sent));
        let sponsor_future_without = sent.contains("sponsor")
            && SPONSOR_FUTURE_WITHOUT_RE.is_match(sent)
            && (sent.contains("without") || sent.contains("not require") || sent.contains("must not"));

        if cpt_refused || citizens_only || sponsor_future_without {
            return Fact {
                value: Sponsorship::NotOffered,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 2. Offered via CPT/OPT. Evaluated per clause: if a clause has cpt/opt + an offered word and no negation inside that clause, it is Offered.
    for sent in &inp.sentences {
        if has_cpt_opt_offered(sent) {
            return Fact {
                value: Sponsorship::Offered,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 3. Asked.
    // Form: any question label containing sponsor, visa status or immigration.
    for q in &inp.questions {
        if q.label.contains("sponsor") || q.label.contains("visa status") || q.label.contains("immigration") {
            return Fact {
                value: Sponsorship::Asked,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    // Description: a sentence with sponsor plus a negation (not, unable, cannot, can't, won't, no ),
    // or sponsor with not available|unavailable|not offered|not provided.
    for sent in &inp.sentences {
        if sent.contains("sponsor") && SPONSOR_NEGATION_RE.is_match(sent) {
            return Fact {
                value: Sponsorship::Asked,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 4. Offered (generic). Description: sponsorship with available|offered|provided|is possible, and no negation in the sentence.
    for sent in &inp.sentences {
        if sent.contains("sponsorship") && SPONSORSHIP_AVAILABLE_RE.is_match(sent) {
            let has_negation = sent.contains("not")
                || sent.contains("no ")
                || sent.contains("unable")
                || sent.contains("cannot")
                || sent.contains("can't")
                || sent.contains("won't")
                || sent.contains("without");
            if !has_negation {
                return Fact {
                    value: Sponsorship::Offered,
                    evidence: ev(Source::Description, sent),
                };
            }
        }
    }

    // 5. Keywords for the Unknown rule: sponsor, h-1b, h1b, \bcpt\b, \bopt\b, f-1, visa.
    // For visa, only count it when the same sentence also has sponsor, status, student or work.
    for sent in &inp.sentences {
        if KEYWORD_SPONSOR_RE.is_match(sent) {
            return Fact {
                value: Sponsorship::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
        if sent.contains("visa") && VISA_RELATED_RE.is_match(sent) {
            return Fact {
                value: Sponsorship::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        if KEYWORD_SPONSOR_RE.is_match(&q.label) {
            return Fact {
                value: Sponsorship::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
        if q.label.contains("visa") && VISA_RELATED_RE.is_match(&q.label) {
            return Fact {
                value: Sponsorship::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }

    Fact {
        value: Sponsorship::NotMentioned,
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::citizenship::citizenship;
    use crate::model::*;

    #[test]
    fn sponsorship_table() {
        use Sponsorship::*;
        let cases = [
            ("Candidates must be authorized to work in the US without sponsorship now or in the future.", NotOffered),
            ("CPT/OPT candidates are not eligible for this role.", NotOffered),
            ("CPT/OPT students are not eligible.", NotOffered),
            ("We will not accept candidates on OPT.", NotOffered),
            ("This position is open only to U.S. citizens and permanent residents.", NotOffered),
            ("Not only US citizens but all students may apply.", NotMentioned),
            ("We are unable to sponsor H-1B visas for this position.", Asked),
            ("We do not sponsor visas.", Asked),
            ("Visa sponsorship is not available.", Asked),
            ("We do not sponsor H-1B. Students on CPT/OPT are welcome to apply.", Offered),
            ("We do not sponsor H-1B, but students on CPT/OPT are eligible.", Offered),
            ("We do not offer sponsorship; however CPT and OPT are accepted.", Offered),
            ("Visa sponsorship is available for this role.", Offered),
            ("Join our team building payment rails with Visa and Mastercard.", NotMentioned),
            ("Opt-in to receive SMS updates.", NotMentioned),
            ("We build robots.", NotMentioned),
            ("Our sponsorship of open source is legendary.", Unknown),
            ("All applicants must be a U.S. citizen or national of the U.S., an alien lawfully admitted for permanent residence, or an alien authorized to work in the U.S. for this employer.", NotMentioned),
        ];
        for (t, want) in cases {
            assert_eq!(sponsorship(&Input::from_text(t)).value, want, "{t}");
        }
    }

    #[test]
    fn form_question_is_asked_not_refused() {
        let page = PortalPage {
            description: "x".repeat(300),
            questions: vec![FormQuestion {
                label: "Will you now or in the future require sponsorship for employment visa status (e.g. H1B status)?".into(),
                options: vec![],
            }],
        };
        let f = sponsorship(&Input::new("SWE Intern", &page));
        assert_eq!(f.value, Sponsorship::Asked);
        assert_eq!(f.evidence.unwrap().source, Source::Form);
    }

    #[test]
    fn authorized_to_work_question_ignored() {
        let page = PortalPage {
            description: "x".repeat(300),
            questions: vec![FormQuestion {
                label: "Are you legally authorized to work in the United States?".into(),
                options: vec![],
            }],
        };
        assert_eq!(sponsorship(&Input::new("t", &page)).value, Sponsorship::NotMentioned);
        assert_eq!(citizenship(&Input::new("t", &page)).value, Citizenship::NotMentioned);
    }
}
