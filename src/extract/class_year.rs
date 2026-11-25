use std::collections::BTreeSet;
use std::sync::LazyLock;
use regex::Regex;

use crate::extract::{ev, Input};
use crate::model::{ClassYears, Fact, Source};

static RISING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\brising\s+(freshm[ea]n\w*|sophomore\w*|junior\w*|senior\w*)\b").expect("valid regex")
});

static RISING_NEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:\s*,\s*(?:or|and)\s*|\s*,\s*|\s+(?:or|and)\s*|\s*/\s*)(freshm[ea]n\w*|sophomore\w*|junior\w*|senior\w*)\b").expect("valid regex")
});

static FRESHMAN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bfreshm[ea]n\w*\b").expect("valid regex")
});

static SOPHOMORE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bsophomore\w*\b").expect("valid regex")
});

static JUNIORS_PLURAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bjuniors\b").expect("valid regex")
});

static JUNIOR_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bjunior\s+(?:year|standing|student)s?\b").expect("valid regex")
});

static JUNIOR_BARE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bjuniors?\b").expect("valid regex")
});

static SENIORS_PLURAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bseniors\b").expect("valid regex")
});

static SENIOR_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bsenior\s+(?:year|standing|student)s?\b").expect("valid regex")
});

static SENIOR_BARE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bseniors?\b").expect("valid regex")
});

static FIRST_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:first|1st)[- ]year\b").expect("valid regex")
});

static SECOND_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:second|2nd)[- ]year\b").expect("valid regex")
});

static THIRD_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:third|3rd)[- ]year\b").expect("valid regex")
});

static FOURTH_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:fourth|4th|final)[- ]year\b").expect("valid regex")
});

static ACADEMIC_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(students?|undergrads?|undergraduate\w*|college\w*|university\w*|degree\w*|bachelor\w*|class\w*|enrolled)\b").expect("valid regex")
});

static GRAD_SENTENCE_TRIGGER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bgraduat|class of").expect("valid regex")
});

static BETWEEN_YEARS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bbetween\b([^.]*?)\b(20\d\d)\b([^.]*?)\band\b([^.]*?)\b(20\d\d)\b").expect("valid regex")
});

static RANGE_YEARS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(20\d\d)\s*(?:-|to)\s*(20\d\d)\b").expect("valid regex")
});

static LATER_YEARS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(20\d\d)\s*(?:\+|or\s+(?:later|after))\b").expect("valid regex")
});

static EARLIER_YEARS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:before\s+(20\d\d)|(20\d\d)\s+or\s+earlier)\b").expect("valid regex")
});

static FOUR_DIGIT_YEAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(20\d\d)\b").expect("valid regex")
});

static EXPLICIT_ANY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(all class years|all years|any year|all levels)\b").expect("valid regex")
});

static FORM_QUESTION_LABEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(graduat|school year|year in school|class standing|academic standing|academic year|class year)").expect("valid regex")
});

static TITLE_PROGRAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(explore|step)\b|freshm[ea]n\s*/\s*sophomore").expect("valid regex")
});

static PREFERENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(preferred|preference|preferably|ideally|a plus|desired|nice to have|typically|usually|generally|encouraged)\b").expect("valid regex")
});

static OTHER_KEYWORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bfreshm[ea]n\w*|\bsophomore\w*|\bgraduat|class of").expect("valid regex")
});

fn rising_standing_year(target: &str, season_year: u16) -> Option<u16> {
    let lower = target.to_lowercase();
    if lower.starts_with("sophomore") {
        Some(season_year + 3)
    } else if lower.starts_with("junior") {
        Some(season_year + 2)
    } else if lower.starts_with("senior") {
        Some(season_year + 1)
    } else if lower.starts_with("freshm") {
        Some(season_year + 4)
    } else {
        None
    }
}

fn count_standing_kinds(text: &str) -> usize {
    let mut count = 0;
    if FRESHMAN_RE.is_match(text) {
        count += 1;
    }
    if SOPHOMORE_RE.is_match(text) {
        count += 1;
    }
    if JUNIORS_PLURAL_RE.is_match(text) || JUNIOR_CONTEXT_RE.is_match(text) {
        count += 1;
    }
    if SENIORS_PLURAL_RE.is_match(text) || SENIOR_CONTEXT_RE.is_match(text) {
        count += 1;
    }
    count
}

fn has_student_context(text: &str) -> bool {
    ACADEMIC_CONTEXT_RE.is_match(text) || count_standing_kinds(text) >= 2
}

fn has_class_year_keyword(text: &str) -> bool {
    if OTHER_KEYWORD_RE.is_match(text) {
        return true;
    }
    if JUNIORS_PLURAL_RE.is_match(text) || SENIORS_PLURAL_RE.is_match(text) {
        return has_student_context(text);
    }
    false
}

fn extract_standing_years(text: &str, season_year: u16, is_sentence: bool) -> BTreeSet<u16> {
    let mut years = BTreeSet::new();
    let mut bytes = text.as_bytes().to_vec();

    // 1. Rising standing words first; mask matched spans
    let mut last_masked_end = 0;
    for cap in RISING_RE.captures_iter(text) {
        let whole = cap.get(0).expect("group 0 exists");
        if whole.start() < last_masked_end {
            continue;
        }
        let target = cap.get(1).expect("group 1 exists").as_str();
        if let Some(yr) = rising_standing_year(target, season_year) {
            years.insert(yr);
        }

        let mut curr_end = whole.end();
        while let Some(next_cap) = RISING_NEXT_RE.captures(&text[curr_end..]) {
            let next_match = next_cap.get(0).expect("group 0 exists");
            let next_target = next_cap.get(1).expect("group 1 exists").as_str();
            if let Some(yr) = rising_standing_year(next_target, season_year) {
                years.insert(yr);
            }
            curr_end += next_match.end();
        }

        let start = whole.start();
        bytes[start..curr_end].fill(b' ');
        last_masked_end = curr_end;
    }

    let masked = String::from_utf8(bytes).unwrap_or_else(|_| text.to_string());

    // 2. Freshman / freshmen -> S+3
    if FRESHMAN_RE.is_match(&masked) {
        years.insert(season_year + 3);
    }

    // 3. Sophomore(s) -> S+2
    if SOPHOMORE_RE.is_match(&masked) {
        years.insert(season_year + 2);
    }

    let has_student_ctx = has_student_context(text);

    // 4. Junior(s)
    if is_sentence {
        if (JUNIORS_PLURAL_RE.is_match(&masked) && has_student_ctx) || JUNIOR_CONTEXT_RE.is_match(&masked) {
            years.insert(season_year + 1);
        }
    } else if JUNIOR_BARE_RE.is_match(&masked) {
        years.insert(season_year + 1);
    }

    // 5. Senior(s)
    if is_sentence {
        if (SENIORS_PLURAL_RE.is_match(&masked) && has_student_ctx) || SENIOR_CONTEXT_RE.is_match(&masked) {
            years.insert(season_year);
            years.insert(season_year + 1);
        }
    } else if SENIOR_BARE_RE.is_match(&masked) {
        years.insert(season_year);
        years.insert(season_year + 1);
    }

    // 6. Nth-year
    let has_academic_context = !is_sentence || ACADEMIC_CONTEXT_RE.is_match(text);
    if has_academic_context {
        if FIRST_YEAR_RE.is_match(&masked) {
            years.insert(season_year + 3);
        }
        if SECOND_YEAR_RE.is_match(&masked) {
            years.insert(season_year + 2);
        }
        if THIRD_YEAR_RE.is_match(&masked) {
            years.insert(season_year + 1);
        }
        if FOURTH_YEAR_RE.is_match(&masked) {
            years.insert(season_year);
            years.insert(season_year + 1);
        }
    }

    years
}

fn parse_grad_years(text: &str, season_year: u16) -> BTreeSet<u16> {
    let mut years = BTreeSet::new();

    // 1. Remove "summer {S}" and "summer of {S}", and two-season ranges ending in {S}
    let summer_pattern = format!(r"\bsummer\s+(?:of\s+)?{}\b", season_year);
    let clean = if let Ok(re) = Regex::new(&summer_pattern) {
        re.replace_all(text, " ").to_string()
    } else {
        text.to_string()
    };
    let season_range_pattern = format!(
        r"\b(spring|summer|fall|autumn|winter)\s*(-|/|and|&|through|to)\s*(spring|summer|fall|autumn|winter)\s+(of\s+)?{}\b",
        season_year
    );
    let clean = if let Ok(re) = Regex::new(&season_range_pattern) {
        re.replace_all(&clean, " ").to_string()
    } else {
        clean
    };

    // 2. between Y1 and Y2
    for cap in BETWEEN_YEARS_RE.captures_iter(&clean) {
        if let (Some(m1), Some(m2)) = (cap.get(2), cap.get(5)) {
            if let (Ok(y1), Ok(y2)) = (m1.as_str().parse::<u16>(), m2.as_str().parse::<u16>()) {
                if (2020..=2040).contains(&y1) && (2020..=2040).contains(&y2) && y1 <= y2 {
                    for y in y1..=y2 {
                        years.insert(y);
                    }
                }
            }
        }
    }

    // 3. Y1-Y2 and Y1 to Y2
    for cap in RANGE_YEARS_RE.captures_iter(&clean) {
        if let (Some(m1), Some(m2)) = (cap.get(1), cap.get(2)) {
            if let (Ok(y1), Ok(y2)) = (m1.as_str().parse::<u16>(), m2.as_str().parse::<u16>()) {
                if (2020..=2040).contains(&y1) && (2020..=2040).contains(&y2) && y1 <= y2 {
                    for y in y1..=y2 {
                        years.insert(y);
                    }
                }
            }
        }
    }

    // 4. Y or later / Y or after
    for cap in LATER_YEARS_RE.captures_iter(&clean) {
        if let Some(m) = cap.get(1) {
            if let Ok(y) = m.as_str().parse::<u16>() {
                if (2020..=2040).contains(&y) {
                    for yr in y..=y + 6 {
                        years.insert(yr);
                    }
                }
            }
        }
    }

    // 5. Y or earlier / before Y
    for cap in EARLIER_YEARS_RE.captures_iter(&clean) {
        let m = cap.get(1).or_else(|| cap.get(2));
        if let Some(m) = m {
            if let Ok(y) = m.as_str().parse::<u16>() {
                if (2020..=2040).contains(&y) && season_year <= y {
                    for yr in season_year..=y {
                        years.insert(yr);
                    }
                }
            }
        }
    }

    // 6. Any other 4-digit years in 2020..=2040
    for cap in FOUR_DIGIT_YEAR_RE.captures_iter(&clean) {
        if let Some(m) = cap.get(1) {
            if let Ok(y) = m.as_str().parse::<u16>() {
                if (2020..=2040).contains(&y) {
                    years.insert(y);
                }
            }
        }
    }

    years
}

pub fn class_years(inp: &Input, season_year: u16) -> Fact<ClassYears> {
    // 1. Form questions
    for q in &inp.questions {
        if FORM_QUESTION_LABEL_RE.is_match(&q.label) {
            // Check for "other" in options
            if q.options.iter().any(|opt| opt.contains("other")) {
                return Fact {
                    value: ClassYears::Any,
                    evidence: ev(Source::Form, &q.label),
                };
            }

            let mut form_years = BTreeSet::new();
            for opt in &q.options {
                if opt.contains("already graduated") {
                    continue;
                }
                form_years.extend(parse_grad_years(opt, season_year));
                form_years.extend(extract_standing_years(opt, season_year, false));
            }

            if !form_years.is_empty() {
                return Fact {
                    value: ClassYears::Grad(form_years),
                    evidence: ev(Source::Form, &q.label),
                };
            }
        }
    }

    // 2. Explicit Any in description sentences
    for sent in &inp.sentences {
        if EXPLICIT_ANY_RE.is_match(sent) {
            return Fact {
                value: ClassYears::Any,
                evidence: ev(Source::Description, sent),
            };
        }
    }

    // 3. Title set
    let mut title_years = BTreeSet::new();
    if TITLE_PROGRAM_RE.is_match(&inp.title) {
        title_years.insert(season_year + 2);
        title_years.insert(season_year + 3);
    }
    title_years.extend(extract_standing_years(&inp.title, season_year, true));

    // Description sentences set
    let mut desc_years = BTreeSet::new();
    let mut desc_ev = None;
    let mut first_preferred_ev = None;
    for sent in &inp.sentences {
        let mut sent_years = BTreeSet::new();
        if GRAD_SENTENCE_TRIGGER_RE.is_match(sent) {
            sent_years.extend(parse_grad_years(sent, season_year));
        }
        sent_years.extend(extract_standing_years(sent, season_year, true));
        if !sent_years.is_empty() {
            if PREFERENCE_RE.is_match(sent) {
                if first_preferred_ev.is_none() {
                    first_preferred_ev = ev(Source::Description, sent);
                }
            } else {
                if desc_ev.is_none() {
                    desc_ev = ev(Source::Description, sent);
                }
                desc_years.extend(sent_years);
            }
        }
    }

    let mut combined_years = desc_years;
    combined_years.extend(title_years);

    if !combined_years.is_empty() {
        let evidence = desc_ev.or_else(|| ev(Source::Title, &inp.title));
        return Fact {
            value: ClassYears::Grad(combined_years),
            evidence,
        };
    }

    if let Some(evidence) = first_preferred_ev {
        return Fact {
            value: ClassYears::Unknown,
            evidence: Some(evidence),
        };
    }

    // 4. Keywords for Unknown
    for sent in &inp.sentences {
        if has_class_year_keyword(sent) {
            return Fact {
                value: ClassYears::Unknown,
                evidence: ev(Source::Description, sent),
            };
        }
    }
    for q in &inp.questions {
        if has_class_year_keyword(&q.label) {
            return Fact {
                value: ClassYears::Unknown,
                evidence: ev(Source::Form, &q.label),
            };
        }
    }

    // 5. Default Any (no evidence)
    Fact {
        value: ClassYears::Any,
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FormQuestion, PortalPage, Source};

    fn cy(t: &str) -> ClassYears {
        class_years(&Input::from_text(t), 2027).value
    }

    fn set(v: &[u16]) -> ClassYears {
        ClassYears::Grad(v.iter().copied().collect())
    }

    #[test]
    fn class_year_table() {
        assert_eq!(cy("Open to freshmen and sophomores."), set(&[2029, 2030]));
        assert_eq!(cy("This program is for rising sophomores."), set(&[2030]));
        assert_eq!(cy("Must be a rising junior or rising senior."), set(&[2028, 2029]));
        assert_eq!(
            cy("Candidates graduating between December 2027 and June 2028."),
            set(&[2027, 2028])
        );
        assert_eq!(
            cy("Class of 2029 or later."),
            ClassYears::Grad((2029..=2035).collect())
        );
        assert_eq!(
            cy("Expected graduation in 2028-2030 for the Summer 2027 internship."),
            set(&[2028, 2029, 2030])
        );
        assert_eq!(cy("Open to students of all class years."), ClassYears::Any);
        assert_eq!(
            cy("Pursuing an undergraduate degree in CS."),
            ClassYears::Any
        ); // "undergraduate" is not "graduat"
        assert_eq!(
            cy("Work alongside senior engineers and junior developers."),
            ClassYears::Any
        );
        assert_eq!(
            cy("First-year students in a bachelor's program are welcome."),
            set(&[2030])
        );
        assert_eq!(cy("Recent graduates welcome."), ClassYears::Unknown);
        assert_eq!(cy("We build robots."), ClassYears::Any);
    }

    #[test]
    fn class_year_form_dropdown_wins() {
        let page = PortalPage {
            description: "Open to juniors and seniors. ".repeat(20),
            questions: vec![FormQuestion {
                label: "If currently enrolled in a program, please select your anticipated graduation year:".into(),
                options: ["2026", "2027", "2028", "2029", "2030", "Already Graduated"]
                    .map(String::from)
                    .to_vec(),
            }],
        };
        let f = class_years(&Input::new("AI Research Fellowship Intern", &page), 2027);
        assert_eq!(f.value, set(&[2026, 2027, 2028, 2029, 2030]));
        assert_eq!(f.evidence.unwrap().source, Source::Form);
    }

    #[test]
    fn class_year_from_title() {
        let page = PortalPage {
            description: "We build things. ".repeat(20),
            questions: vec![],
        };
        assert_eq!(
            class_years(&Input::new("Explore Intern - Software", &page), 2027).value,
            set(&[2029, 2030])
        );
        assert_eq!(
            class_years(&Input::new("Senior Software Engineer Intern", &page), 2027).value,
            ClassYears::Any
        );
    }

    #[test]
    fn class_year_false_no_cases() {
        assert_eq!(
            cy("We provide seniors, patients recovering post-discharge and those managing a chronic condition with tailored nutrition solutions to manage their specific needs."),
            ClassYears::Any
        );
        assert_eq!(
            cy("Direct Supply is building the future of healthcare technology with industry-leading products, solutions and platforms to help improve the lives of millions of seniors and those who care for them."),
            ClassYears::Any
        );
        assert_eq!(
            cy("Open to juniors and seniors."),
            set(&[2027, 2028])
        );
        assert_eq!(
            cy("Juniors or seniors pursuing a degree in computer science."),
            set(&[2027, 2028])
        );
        assert_eq!(
            cy("Must be a rising junior, senior, or master's student."),
            set(&[2028, 2029])
        );
        assert_eq!(
            cy("Open to rising sophomores and juniors."),
            set(&[2029, 2030])
        );
        assert_eq!(
            cy("Rising sophomores and juniors are encouraged to apply."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Successful applicants will typically be third or fourth year undergraduate students, graduate students, or recent graduates, and will be asked to complete a technical assessment in order to show competence in C++, Java, or JavaScript."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Rising seniors majoring in computer science, computer engineering, or a related field with strong academic performance are encouraged to apply."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Open to rising seniors. Juniors are encouraged to apply."),
            set(&[2028])
        );
        assert_eq!(
            cy("This graduate internship is a full-time position summer-fall 2027 (May or June through December) on site in Rochester, Minnesota."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Expected graduation date: spring 2028."),
            set(&[2028])
        );
        assert_eq!(
            cy("Graduating fall 2027 or spring 2028."),
            set(&[2027, 2028])
        );
        assert_eq!(
            cy("Internship runs summer/fall 2027 for students graduating in 2029."),
            set(&[2029])
        );
        assert_eq!(
            cy("Rising Junior, Senior, or Master's student preferred."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Rising sophomore or junior preferred."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("2030 graduation year preferred."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Must be pursuing a degree in computer science, IT, or a related degree (juniors or seniors preferred)."),
            ClassYears::Unknown
        );
        assert_eq!(
            cy("Open to rising seniors. Sophomore or junior standing preferred."),
            set(&[2028])
        );

        let fact_2030 = class_years(&Input::from_text("2030 graduation year preferred."), 2027);
        assert_eq!(fact_2030.value, ClassYears::Unknown);
        assert!(fact_2030.evidence.is_some());
        assert_eq!(fact_2030.evidence.unwrap().source, Source::Description);

        let moms_meals = "We provide seniors, patients recovering post-discharge and those managing a chronic condition with tailored nutrition solutions to manage their specific needs. ";
        let page = PortalPage {
            description: format!("{}{}", moms_meals, "We build things. ".repeat(20)),
            questions: vec![FormQuestion {
                label: "What will your current academic standing be at the time this internship begins?".into(),
                options: ["Freshman", "Sophomore", "Junior", "Senior", "Graduate student", "Recently graduated", "Other / Not applicable"]
                    .map(String::from)
                    .to_vec(),
            }],
        };
        let f = class_years(&Input::new("Data Analytics Intern", &page), 2027);
        assert_eq!(f.value, ClassYears::Any);
        assert_eq!(f.evidence.unwrap().source, Source::Form);
    }
}
