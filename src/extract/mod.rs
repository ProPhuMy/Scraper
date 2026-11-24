#![allow(dead_code)]

pub mod class_year;
pub mod citizenship;
pub mod experience;
pub mod gpa;
pub mod sponsorship;

pub use class_year::class_years;
pub use citizenship::citizenship;
pub use experience::experience;
pub use gpa::gpa;
pub use sponsorship::sponsorship;

use crate::model::{Evidence, Facts, FormQuestion, PortalPage, Source};
use crate::text::{normalize, sentences, snippet};

#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub title: String,
    pub sentences: Vec<String>,
    pub questions: Vec<FormQuestion>,
}

impl Input {
    pub fn new(title: &str, page: &PortalPage) -> Input {
        let norm_title = normalize(title);
        let norm_desc = normalize(&page.description);
        let raw_sentences = sentences(&norm_desc);
        let sentences = raw_sentences
            .into_iter()
            .filter(|s| {
                !s.contains("regard to")
                    && !s.contains("regardless of")
                    && !s.contains("equal opportunity")
                    && !s.contains("discriminat")
                    && !s.contains("protected veteran")
                    && !s.contains("e-verify")
            })
            .collect();

        let questions = page
            .questions
            .iter()
            .map(|q| FormQuestion {
                label: normalize(&q.label),
                options: q.options.iter().map(|o| normalize(o)).collect(),
            })
            .collect();

        Input {
            title: norm_title,
            sentences,
            questions,
        }
    }

    pub fn from_text(desc: &str) -> Input {
        Input::new(
            "",
            &PortalPage {
                description: desc.to_string(),
                questions: Vec::new(),
            },
        )
    }
}

pub fn ev(source: Source, s: &str) -> Option<Evidence> {
    Some(Evidence {
        source,
        snippet: snippet(s),
    })
}

pub fn extract(title: &str, page: &PortalPage, season_year: u16) -> Facts {
    if page.description.chars().count() < 200 && page.questions.is_empty() {
        return Facts::unknown();
    }
    let inp = Input::new(title, page);
    Facts {
        class_years: class_years(&inp, season_year),
        gpa: gpa(&inp),
        sponsorship: sponsorship(&inp),
        citizenship: citizenship(&inp),
        experience: experience(&inp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_page_all_unknown() {
        let page = PortalPage {
            description: "short".into(),
            questions: vec![],
        };
        assert_eq!(extract("t", &page, 2027), Facts::unknown());
    }
}
