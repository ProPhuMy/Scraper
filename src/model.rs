#![allow(dead_code)]

use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Greenhouse,
    Ashby,
    Lever,
    Workday,
    Other,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Greenhouse => "greenhouse",
            Provider::Ashby => "ashby",
            Provider::Lever => "lever",
            Provider::Workday => "workday",
            Provider::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Provider {
        match s {
            "greenhouse" => Provider::Greenhouse,
            "ashby" => Provider::Ashby,
            "lever" => Provider::Lever,
            "workday" => Provider::Workday,
            _ => Provider::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewJob {
    pub simplify_id: String,
    pub company: String,
    pub title: String,
    pub category: String,
    pub url: String,
    pub provider: Provider,
    pub locations: Vec<String>,
    pub date_posted: i64,
    pub date_updated: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub simplify_id: String,
    pub company: String,
    pub title: String,
    pub url: String,
    pub provider: Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormQuestion {
    pub label: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalPage {
    pub description: String,
    pub questions: Vec<FormQuestion>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Page(PortalPage),
    Gone,
    Unreadable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Form,
    Description,
    Title,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: Source,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "years", rename_all = "snake_case")]
pub enum ClassYears {
    Grad(BTreeSet<u16>),
    Any,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "min", rename_all = "snake_case")]
pub enum Gpa {
    Min(f32),
    Asked,
    NotMentioned,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sponsorship {
    Offered,
    NotOffered,
    Asked,
    NotMentioned,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Citizenship {
    Required,
    NotMentioned,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Experience {
    Required,
    Preferred,
    NotMentioned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact<T> {
    pub value: T,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub class_years: Fact<ClassYears>,
    pub gpa: Fact<Gpa>,
    pub sponsorship: Fact<Sponsorship>,
    pub citizenship: Fact<Citizenship>,
    pub experience: Fact<Experience>,
}

impl Facts {
    pub fn unknown() -> Facts {
        Facts {
            class_years: Fact {
                value: ClassYears::Unknown,
                evidence: None,
            },
            gpa: Fact {
                value: Gpa::Unknown,
                evidence: None,
            },
            sponsorship: Fact {
                value: Sponsorship::Unknown,
                evidence: None,
            },
            citizenship: Fact {
                value: Citizenship::Unknown,
                evidence: None,
            },
            experience: Fact {
                value: Experience::Unknown,
                evidence: None,
            },
        }
    }

    pub fn has_unknown(&self) -> bool {
        !self.unknown_names().is_empty()
    }

    pub fn unknown_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if matches!(self.class_years.value, ClassYears::Unknown) {
            names.push("class_years");
        }
        if matches!(self.gpa.value, Gpa::Unknown) {
            names.push("gpa");
        }
        if matches!(self.sponsorship.value, Sponsorship::Unknown) {
            names.push("sponsorship");
        }
        if matches!(self.citizenship.value, Citizenship::Unknown) {
            names.push("citizenship");
        }
        if matches!(self.experience.value, Experience::Unknown) {
            names.push("experience");
        }
        names
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    RulesDone,
    LlmDone,
    Unreadable,
    Gone,
    Error,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::RulesDone => "rules_done",
            Status::LlmDone => "llm_done",
            Status::Unreadable => "unreadable",
            Status::Gone => "gone",
            Status::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "pending" => Status::Pending,
            "rules_done" => Status::RulesDone,
            "llm_done" => Status::LlmDone,
            "unreadable" => Status::Unreadable,
            "gone" => Status::Gone,
            _ => Status::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_roundtrip_json() {
        let mut years = BTreeSet::new();
        years.insert(2029);
        years.insert(2030);

        let facts = Facts {
            class_years: Fact {
                value: ClassYears::Grad(years),
                evidence: Some(Evidence {
                    source: Source::Description,
                    snippet: "Graduating 2029 or 2030".to_string(),
                }),
            },
            gpa: Fact {
                value: Gpa::Min(3.0),
                evidence: Some(Evidence {
                    source: Source::Form,
                    snippet: "Minimum 3.0 GPA".to_string(),
                }),
            },
            sponsorship: Fact {
                value: Sponsorship::Offered,
                evidence: None,
            },
            citizenship: Fact {
                value: Citizenship::NotMentioned,
                evidence: None,
            },
            experience: Fact {
                value: Experience::Required,
                evidence: None,
            },
        };

        let json = serde_json::to_string(&facts).unwrap();
        let roundtrip: Facts = serde_json::from_str(&json).unwrap();
        assert_eq!(facts, roundtrip);
    }

    #[test]
    fn unknown_names_lists_in_order() {
        let mut facts = Facts::unknown();
        facts.gpa.value = Gpa::NotMentioned;
        assert_eq!(
            facts.unknown_names(),
            vec!["class_years", "sponsorship", "citizenship", "experience"]
        );
    }

    #[test]
    fn status_roundtrip() {
        let all_statuses = [
            Status::Pending,
            Status::RulesDone,
            Status::LlmDone,
            Status::Unreadable,
            Status::Gone,
            Status::Error,
        ];

        for s in all_statuses {
            assert_eq!(Status::parse(s.as_str()), s);
        }
        assert_eq!(Status::parse("garbage"), Status::Error);
    }
}
