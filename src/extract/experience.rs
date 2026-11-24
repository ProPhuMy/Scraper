use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Experience, Fact, Source};

static NOT_REQUIRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bno\s+(?:prior\s+|previous\s+)?experience\s+(?:required|necessary|needed)\b|\bexperience\s+is\s+not\s+required\b")
        .expect("valid regex")
});

static EXPERIENCE_NOUN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(internship|work|industry|professional|prior|previous|relevant work)\s+experience\b")
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
    Regex::new(r"\b\d+\+?\s+years?\s+of\s+(?:professional\s+|industry\s+|relevant\s+|work\s+)?experience\b")
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

pub fn experience(inp: &Input) -> Fact<Experience> {
    // 1. Not required. no (prior |previous )?experience (required|necessary|needed) or experience is not required -> NotMentioned, with evidence.
    for sent in &inp.sentences {
        if NOT_REQUIRED_RE.is_match(sent) {
            return Fact {
                value: Experience::NotMentioned,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 2. Preferred. An experience noun with preferred|a plus|bonus|nice to have|desired|desirable.
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
        let has_exp_signal = EXPERIENCE_NOUN_RE.is_match(sent) || YEARS_OF_EXP_RE.is_match(sent);
        if has_exp_signal && EXP_PREFERRED_WORDS_RE.is_match(sent) {
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
        let years_required = YEARS_OF_EXP_RE.is_match(sent);
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
        if EXPERIENCE_NOUN_RE.is_match(sent) || YEARS_OF_EXP_KEYWORD_RE.is_match(sent) {
            return Fact {
                value: Experience::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        if EXPERIENCE_NOUN_RE.is_match(&q.label) || YEARS_OF_EXP_KEYWORD_RE.is_match(&q.label) {
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
        ];
        for (t, want) in cases {
            assert_eq!(experience(&Input::from_text(t)).value, want, "{t}");
        }
    }
}
