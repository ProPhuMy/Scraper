use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Experience, Fact, Source};

static NOT_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bno\s+(?:[\w-]+\s+){0,8}(?:(?:internship|work|industry|professional|prior|previous|relevant work|relevant)\s+)?experience\b(?:\s+[\w-]+){0,3}\s+(?:required|necessary|needed)\b|\b(?:(?:internship|work|industry|professional|prior|previous|relevant work|relevant)\s+)?experience\b.*?\b(?:is\s+)?not\s+(?:required|necessary|needed)\b")
        .expect("valid regex")
});

static ZERO_YEARS_OF_EXP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b0(?:\s*(?:[-–—]|to)\s*\d+|\+)?\s+years?\s+of\s+(?:professional\s+|industry\s+|relevant\s+|work\s+)?experience\b")
        .expect("valid regex")
});

static EXP_CAP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(no more than|less than|fewer than|up to|at most|maximum of)\s+\d+(?:\.\d+)?\s+years?\b")
        .expect("valid regex")
});

static EXP_ALTERNATIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bexperience\b(?:\s+[\w-]+){0,3}\s+or\s+(?:[\w-]+\s+){0,3}(?:education|coursework|training|degrees?|academic)\b|\b(?:education|coursework|training|degrees?|academic)\b.*?\bor\b(?:\s+[\w-]+){0,4}\s+(?:(?:internship|work|industry|professional|prior|previous|relevant work|relevant)\s+)?experience\b")
        .expect("valid regex")
});

static HEDGED_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(could|may|might|can)\s+be\b.*?\b(required|requires|necessary|needed)\b")
        .expect("valid regex")
});

static EXPERIENCE_NOUN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(internship|work|industry|professional|prior|previous|relevant work|relevant)\s+experience\b")
        .expect("valid regex")
});

static EXP_PREFERRED_WORDS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(preferred|a plus|bonus|nice to have|desired|desirable)\b")
        .expect("valid regex")
});

static FORM_EXP_PREFERRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(previous internship|prior internship|internship experience)\b")
        .expect("valid regex")
});

static YEARS_OF_EXP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([1-9]\d*)(?:\s*(?:[-–—]|to)\s*\d+|\+)?\s+years?\s+of\s+(?:professional\s+|industry\s+|relevant\s+|work\s+)?experience\b")
        .expect("valid regex")
});

static EXP_REQUIRED_WORDS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(required|must have|must possess|requires|minimum)\b")
        .expect("valid regex")
});

static MUST_HAVE_INTERNSHIP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bmust have completed\s+(at least one|an|a previous)\s+internship\b")
        .expect("valid regex")
});

static YEARS_OF_EXP_KEYWORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\byears?\s+of\s+experience\b").expect("valid regex")
});

fn is_company_boilerplate_years(sent: &str) -> bool {
    for m in YEARS_OF_EXP_RE.captures_iter(sent) {
        if let Some(num) = m.get(1) {
            if let Ok(n) = num.as_str().parse::<u32>() {
                let before = &sent[..num.start()];
                let has_we_our = (before.contains("we ") || before.contains("our ")) && !before.contains("require");
                if n > 10 || has_we_our {
                    return true;
                }
            }
        }
    }
    false
}

fn has_valid_years_of_exp(sent: &str) -> bool {
    for m in YEARS_OF_EXP_RE.captures_iter(sent) {
        if let Some(num) = m.get(1) {
            if let Ok(n) = num.as_str().parse::<u32>() {
                if n > 10 {
                    continue;
                }
                let before = &sent[..num.start()];
                let has_we_our = (before.contains("we ") || before.contains("our ")) && !before.contains("require");
                if has_we_our {
                    continue;
                }
                return true;
            }
        }
    }
    false
}

pub fn experience(inp: &Input) -> Fact<Experience> {
    // 1. Not required (checked first).
    // - "no" followed within up to 8 words by an experience phrase followed within 3 words by required|necessary|needed -> NotMentioned
    // - <experience noun> ... is not required|is not necessary|not needed -> NotMentioned
    // - years-of-experience starting at 0 -> NotMentioned
    // - cap before N years ("no more than", "less than", "fewer than", "up to", "at most", "maximum of") -> NotMentioned
    // - company boilerplate years (N > 10 or "we " / "our " before the number) -> NotMentioned
    // - experience phrase joined by "or" to education/coursework/training/degree/academic -> NotMentioned
    for sent in &inp.sentences {
        let is_cap = EXP_CAP_RE.is_match(sent) && sent.contains("experience");
        let is_boilerplate = is_company_boilerplate_years(sent);
        let is_alternative = EXP_ALTERNATIVE_RE.is_match(sent);

        if NOT_REQUIRED_RE.is_match(sent)
            || ZERO_YEARS_OF_EXP_RE.is_match(sent)
            || is_cap
            || is_boilerplate
            || is_alternative
        {
            return Fact {
                value: Experience::NotMentioned,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 2. Preferred. An experience noun with preferred|a plus|bonus|nice to have|desired|desirable.
    // Or hedged required: could be / may be / might be / can be before required.
    // Or N years of experience ... preferred/a plus.
    // Form: any question label with previous internship, prior internship or internship experience.
    for q in &inp.questions {
        if FORM_EXP_PREFERRED_RE.is_match(&q.label) {
            return Fact {
                value: Experience::Preferred,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    for sent in &inp.sentences {
        let has_exp_noun = EXPERIENCE_NOUN_RE.is_match(sent);
        let has_years = has_valid_years_of_exp(sent);
        let is_preferred = (has_exp_noun || has_years) && EXP_PREFERRED_WORDS_RE.is_match(sent);
        let is_hedged = (has_exp_noun || has_years) && HEDGED_REQUIRED_RE.is_match(sent);

        if is_preferred || is_hedged {
            return Fact {
                value: Experience::Preferred,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 3. Required.
    // - An experience noun with required|must have|must possess|requires|minimum.
    // - Or \d+\+? years? of (professional |industry |relevant |work )?experience, unless the sentence also says preferred or a plus.
    // - Or must have completed (at least one|an|a previous) internship.
    for sent in &inp.sentences {
        let noun_required = EXPERIENCE_NOUN_RE.is_match(sent) && EXP_REQUIRED_WORDS_RE.is_match(sent);
        let years_required = has_valid_years_of_exp(sent);
        let completed_internship = MUST_HAVE_INTERNSHIP_RE.is_match(sent);

        if noun_required || years_required || completed_internship {
            return Fact {
                value: Experience::Required,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 4. Keywords: the experience nouns above, and years of experience.
    for sent in &inp.sentences {
        let has_years = has_valid_years_of_exp(sent) || (!is_company_boilerplate_years(sent) && YEARS_OF_EXP_KEYWORD_RE.is_match(sent));
        if EXPERIENCE_NOUN_RE.is_match(sent) || has_years {
            return Fact {
                value: Experience::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        let has_years = has_valid_years_of_exp(&q.label) || (!is_company_boilerplate_years(&q.label) && YEARS_OF_EXP_KEYWORD_RE.is_match(&q.label));
        if EXPERIENCE_NOUN_RE.is_match(&q.label) || has_years {
            return Fact {
                value: Experience::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }

    // 5. Not mentioned.
    Fact {
        value: Experience::NotMentioned,
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_table() {
        use Experience::*;
        let cases = [
            ("Prior internship experience is required.", Required),
            ("2+ years of professional experience.", Required),
            ("Must have completed at least one internship.", Required),
            ("Previous internship experience is a plus.", Preferred),
            ("Relevant work experience preferred.", Preferred),
            ("No prior experience required!", NotMentioned),
            ("Experience with Python or Java required.", NotMentioned),
            ("Experience in C++ is a plus.", NotMentioned),
            ("We value industry experience.", Unknown),
            ("We build robots.", NotMentioned),
            ("No prior work experience required.", NotMentioned),
            ("No previous internship experience is necessary.", NotMentioned),
            ("Prior work experience is not required.", NotMentioned),
            ("0-2 years of experience.", NotMentioned),
            ("0+ years of professional experience.", NotMentioned),
            ("0 to 1 years of experience preferred.", NotMentioned),
            ("1-3 years of industry experience required.", Required),
            ("* no finance or economics study or work experience required", NotMentioned),
            ("Must have no more than 2 years of professional experience in a similar role.", NotMentioned),
            ("Less than 1 year of work experience.", NotMentioned),
            ("Up to 2 years of industry experience.", NotMentioned),
            ("At most 1 year of professional experience.", NotMentioned),
            ("Experience: minimum 1 year of relevant experience required", Required),
            ("drawing on more than 150 years of experience, we are leading the way in safety, efficiency, reliability, innovation, and productivity", NotMentioned),
            ("our engineers average more than 16 years of experience, and teaching is part of how we work", NotMentioned),
            ("required field of study: a field relevant to the role as demonstrated through coursework, training, or professional experience", NotMentioned),
            ("- requires basic job knowledge of systems and procedures obtained through prior work experience or education", NotMentioned),
            ("- course or internship experience related to the following areas could be required: relational databases, linear algebra", Preferred),
            ("- 2+ years of experience in data-related technologies and architecture", Required),
            ("learning role with 1 to 2 years of experience", Required),
            ("- at least 1 year of experience building backend solutions with java", Required),
            ("· prior experience in data science required", Required),
        ];
        for (t, want) in cases {
            assert_eq!(experience(&Input::from_text(t)).value, want, "{t}");
        }
    }
}
