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

use std::sync::LazyLock;
use regex::Regex;

use crate::model::{Evidence, Facts, FormQuestion, PortalPage, Source};
use crate::text::{normalize, sentences, snippet};

static LOCATION_LABEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:which|prefer|preference|interested in|closest to)\b.{0,40}\b(?:location|office|city|site)\b|\b(?:location|office)\s+preference\b)")
        .expect("valid regex")
});

pub fn asks_location_choice(questions: &[FormQuestion], locations: &[String]) -> bool {
    let mut unique_cities: Vec<String> = locations
        .iter()
        .filter_map(|loc| {
            let city = match loc.split_once(',') {
                Some((c, _)) => c.trim().to_lowercase(),
                None => loc.trim().to_lowercase(),
            };
            if city.is_empty() {
                None
            } else {
                Some(city)
            }
        })
        .collect();
    unique_cities.sort();
    unique_cities.dedup();

    for q in questions {
        let label_lower = q.label.to_lowercase();
        if label_lower.contains("current location") {
            continue;
        }

        if LOCATION_LABEL_RE.is_match(&q.label) {
            return true;
        }

        let matched_cities_count = unique_cities
            .iter()
            .filter(|city| {
                q.options.iter().any(|opt| {
                    let opt_clean = opt.trim().to_lowercase();
                    opt_clean.starts_with(city.as_str())
                        || (city.chars().count() >= 4 && opt_clean.contains(city.as_str()))
                })
            })
            .count();

        if matched_cities_count >= 2 {
            return true;
        }
    }

    false
}

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

    #[test]
    fn asks_location_table() {
        let locs = vec!["Atlanta, GA".to_string(), "Boston, MA".into(), "Costa Mesa, CA".into()];
        let q = |label: &str, opts: &[&str]| FormQuestion { label: label.into(), options: opts.iter().map(|s| s.to_string()).collect() };
        assert!(asks_location_choice(&[q("What is your top location preference?", &["Atlanta, GA","Boston, MA","Broomfield, CO"])], &locs));
        assert!(asks_location_choice(&[q("Which MORSE office would you prefer to work in?", &[])], &locs));
        assert!(asks_location_choice(&[q("Which location are you the closest to?", &[])], &locs));
        assert!(asks_location_choice(&[q("Location Preference", &["Chicago","London"])], &vec!["London, UK".into(), "Chicago, IL".into()]));
        assert!(!asks_location_choice(&[q("Current Location", &[])], &locs));
        assert!(!asks_location_choice(&[q("Website", &[])], &locs));
        assert!(!asks_location_choice(&[q("Are you willing to relocate?", &["Yes","No"])], &locs));
    }
}
