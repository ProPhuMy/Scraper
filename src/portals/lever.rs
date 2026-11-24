#![allow(dead_code)]

use std::sync::LazyLock;
use scraper::{ElementRef, Html, Node, Selector};
use crate::model::{Fetched, FormQuestion, Job, PortalPage};
use crate::portals::Http;

static QUESTION_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("li.application-question.custom-question").expect("valid selector"));
static LABEL_TEXT_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".application-label .text").expect("valid selector"));
static LABEL_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".application-label").expect("valid selector"));
static MC_LABEL_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("ul[data-qa=\"multiple-choice\"] label, ul[data-qa=\"checkboxes\"] label").expect("valid selector"));
static SELECT_OPTION_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("select option").expect("valid selector"));

pub fn ids(job_url: &str) -> Option<(String, String, String)> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    let api_host = match host {
        "jobs.lever.co" => "api.lever.co",
        "jobs.eu.lever.co" => "api.eu.lever.co",
        _ => return None,
    };

    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() == 2 {
        Some((api_host.to_string(), segments[0].to_string(), segments[1].to_string()))
    } else if segments.len() == 3 && segments[2] == "apply" {
        Some((api_host.to_string(), segments[0].to_string(), segments[1].to_string()))
    } else {
        None
    }
}

pub fn parse_api(json: &str) -> anyhow::Result<String> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let mut sections = Vec::new();

    let desc = match v.get("descriptionPlain").and_then(|d| d.as_str()).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => v
            .get("description")
            .and_then(|d| d.as_str())
            .map(|d| crate::text::html_to_text(d))
            .unwrap_or_default(),
    };
    let desc_trimmed = desc.trim();
    if !desc_trimmed.is_empty() {
        sections.push(desc_trimmed.to_string());
    }

    if let Some(lists) = v.get("lists").and_then(|l| l.as_array()) {
        for item in lists {
            let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
            let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let content_text = crate::text::html_to_text(content);
            let content_trimmed = content_text.trim();
            if !text.is_empty() && !content_trimmed.is_empty() {
                sections.push(format!("{text}\n{content_trimmed}"));
            } else if !text.is_empty() {
                sections.push(text.to_string());
            } else if !content_trimmed.is_empty() {
                sections.push(content_trimmed.to_string());
            }
        }
    }

    let additional = match v.get("additionalPlain").and_then(|a| a.as_str()).map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => v
            .get("additional")
            .and_then(|a| a.as_str())
            .map(|a| crate::text::html_to_text(a))
            .unwrap_or_default(),
    };
    let additional_trimmed = additional.trim();
    if !additional_trimmed.is_empty() {
        sections.push(additional_trimmed.to_string());
    }

    Ok(sections.join("\n"))
}

fn extract_text_excluding_required(el: ElementRef<'_>) -> String {
    let mut out = String::new();
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                out.push_str(t);
            }
            Node::Element(elem) => {
                if !elem.classes().any(|c| c == "required") {
                    if let Some(child_el) = ElementRef::wrap(child) {
                        out.push_str(&extract_text_excluding_required(child_el));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn parse_apply(html: &str) -> Vec<FormQuestion> {
    let doc = Html::parse_document(html);
    let mut questions = Vec::new();

    for q_el in doc.select(&QUESTION_SEL) {
        let raw_label = if let Some(text_el) = q_el.select(&LABEL_TEXT_SEL).next() {
            extract_text_excluding_required(text_el)
        } else if let Some(label_el) = q_el.select(&LABEL_SEL).next() {
            extract_text_excluding_required(label_el)
        } else {
            String::new()
        };

        let label = raw_label
            .trim()
            .trim_end_matches(['✱', '*'])
            .trim()
            .to_string();

        let mut options = Vec::new();
        for opt_el in q_el.select(&MC_LABEL_SEL) {
            let text = opt_el.text().collect::<String>();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                options.push(trimmed.to_string());
            }
        }
        for opt_el in q_el.select(&SELECT_OPTION_SEL) {
            let text = opt_el.text().collect::<String>();
            let trimmed = text.trim();
            if !trimmed.is_empty() && trimmed != "Select..." {
                options.push(trimmed.to_string());
            }
        }

        questions.push(FormQuestion { label, options });
    }

    questions
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    let (api_host, company, id) = match ids(&job.url) {
        Some(t) => t,
        None => return crate::portals::generic::fetch(client, job).await,
    };

    let api_url = format!("https://{api_host}/v0/postings/{company}/{id}");
    let description = match crate::portals::get(client, &api_url).await? {
        Http::Ok(json) => parse_api(&json)?,
        Http::Gone => return Ok(Fetched::Gone),
        Http::Blocked(code) => return Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    };

    let jobs_host = if api_host == "api.eu.lever.co" {
        "jobs.eu.lever.co"
    } else {
        "jobs.lever.co"
    };
    let apply_url = format!("https://{jobs_host}/{company}/{id}/apply");

    let questions = match crate::portals::get(client, &apply_url).await {
        Ok(Http::Ok(html)) => parse_apply(&html),
        _ => Vec::new(),
    };

    Ok(Fetched::Page(PortalPage {
        description,
        questions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lever_ids() {
        assert_eq!(
            ids("https://jobs.lever.co/solopulseco/418b3949-3b21-48f9-b7a0-1c036ca0ba51/apply"),
            Some((
                "api.lever.co".into(),
                "solopulseco".into(),
                "418b3949-3b21-48f9-b7a0-1c036ca0ba51".into()
            ))
        );
        assert_eq!(
            ids("https://jobs.eu.lever.co/quantinuum/ef4b4db2-4ce1-47f5-bfd1-12dfc8a90107/apply")
                .unwrap()
                .0,
            "api.eu.lever.co"
        );
        assert_eq!(
            ids("https://jobs.lever.co/go-cloudforce/cf2f1855-3ad7-4d4a-a100-2e627f703af4")
                .unwrap()
                .1,
            "go-cloudforce"
        );
        assert_eq!(ids("https://jobs.lever.co/solopulseco"), None);
    }

    #[test]
    fn lever_api_description() {
        let d = parse_api(include_str!("../../tests/fixtures/lever_api.json")).unwrap();
        assert!(d.contains("SoloPulse"));
    }

    #[test]
    fn lever_apply_questions() {
        let qs = parse_apply(include_str!("../../tests/fixtures/lever_apply.html"));
        let q = qs
            .iter()
            .find(|q| q.label == "Are you capable of holding a United States security clearance?")
            .unwrap();
        assert!(!q.options.is_empty());
        assert!(qs
            .iter()
            .any(|q| q.label.starts_with("Are you willing to work and live in Peachtree Corners")));
        assert!(qs.iter().all(|q| !q.label.ends_with('✱')));
    }
}
