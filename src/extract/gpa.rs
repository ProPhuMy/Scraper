use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{Fact, Gpa, Source};

static GPA_KEYWORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(gpa|grade point)\b").expect("valid regex")
});

static GPA_PREFERRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(preferred|a plus|desired|recommended)\b").expect("valid regex")
});

static GPA_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d\.\d{1,2}\b").expect("valid regex")
});

pub fn gpa(inp: &Input) -> Fact<Gpa> {
    // 1. Preferred, not required. A sentence mentioning GPA with preferred|a plus|desired|recommended -> Asked.
    for sent in &inp.sentences {
        if GPA_KEYWORD_RE.is_match(sent) && GPA_PREFERRED_RE.is_match(sent) {
            return Fact {
                value: Gpa::Asked,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 2. Minimum. A description sentence with \bgpa\b or grade point containing a number \d\.\d{1,2}
    // with value in [1.0, 4.0]. Take the first such number that is not immediately preceded by /
    // (so 3.2/4.0 -> 3.2) -> Min(x).
    for sent in &inp.sentences {
        if GPA_KEYWORD_RE.is_match(sent) {
            for m in GPA_NUMBER_RE.find_iter(sent) {
                if sent[..m.start()].trim_end().ends_with('/') {
                    continue;
                }
                if let Ok(val) = m.as_str().parse::<f32>() {
                    if (1.0..=4.0).contains(&val) {
                        return Fact {
                            value: Gpa::Min(val),
                            evidence: ev(Source::Description, sent),
                        };
                    }
                }
            }
        }
    }

    // 3. Asked.
    // Form: a question label with \bgpa\b or grade point. Its options never set a minimum.
    for q in &inp.questions {
        if GPA_KEYWORD_RE.is_match(&q.label) {
            return Fact {
                value: Gpa::Asked,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }
    // Description: a sentence with \bgpa\b/grade point and no valid number.
    for sent in &inp.sentences {
        if GPA_KEYWORD_RE.is_match(sent) {
            return Fact {
                value: Gpa::Asked,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 4. Keywords: \bgpa\b, grade point. Any match is already covered by Asked, so GPA has no Unknown from keywords.
    Fact {
        value: Gpa::NotMentioned,
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FormQuestion, PortalPage};

    #[test]
    fn gpa_table() {
        use Gpa::*;
        let cases = [
            ("Minimum 3.0 GPA required.", Min(3.0)),
            ("Must have a cumulative GPA of 3.2/4.0 or higher.", Min(3.2)),
            ("GPA: 3.5+", Min(3.5)),
            ("A GPA of 3.5 or above is preferred.", Asked),
            ("Please include your GPA on your resume.", Asked),
            ("Maintains a 4.3 GPA scale", Asked),
            ("We build robots in 3.0 seconds.", NotMentioned),
            ("We build robots.", NotMentioned),
        ];
        for (t, want) in cases {
            assert_eq!(gpa(&Input::from_text(t)).value, want, "{t}");
        }
    }

    #[test]
    fn gpa_form_question_is_asked() {
        let page = PortalPage {
            description: "x".repeat(300),
            questions: vec![FormQuestion {
                label: "Please list your current or most recent GPA:".into(),
                options: vec![],
            }],
        };
        assert_eq!(gpa(&Input::new("t", &page)).value, Gpa::Asked);
    }
}
