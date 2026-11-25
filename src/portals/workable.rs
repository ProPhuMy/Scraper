#![allow(dead_code)]

use crate::model::{Fetched, FormQuestion, Job, PortalPage};
use crate::portals::Http;

pub fn ids(job_url: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    if host != "apply.workable.com" {
        return None;
    }
    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() == 3 && segments[1] == "j" {
        Some((segments[0].to_string(), segments[2].to_string()))
    } else if segments.len() == 4 && segments[1] == "j" && segments[3] == "apply" {
        Some((segments[0].to_string(), segments[2].to_string()))
    } else {
        None
    }
}

fn parse_form_questions(form_json: &str) -> Vec<FormQuestion> {
    let form_val: serde_json::Value = match serde_json::from_str(form_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let sections = match form_val.as_array() {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut questions = Vec::new();
    for section in sections {
        if let Some(fields) = section.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                let is_qa = field
                    .get("id")
                    .and_then(|i| i.as_str())
                    .map(|id| id.starts_with("QA_"))
                    .unwrap_or(false);
                if !is_qa {
                    continue;
                }
                let label = field.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string();
                let field_type = field.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let options = if field_type == "boolean" {
                    vec!["Yes".to_string(), "No".to_string()]
                } else if let Some(opts) = field.get("options").and_then(|o| o.as_array()) {
                    opts.iter()
                        .filter_map(|o| o.get("value").and_then(|v| v.as_str()))
                        .filter(|v| !v.is_empty())
                        .map(|v| v.to_string())
                        .collect()
                } else {
                    Vec::new()
                };
                questions.push(FormQuestion { label, options });
            }
        }
    }
    questions
}

pub fn parse(job_json: &str, form_json: Option<&str>) -> Fetched {
    let v: serde_json::Value = match serde_json::from_str(job_json) {
        Ok(v) => v,
        Err(_) => return Fetched::Unreadable("invalid JSON".to_string()),
    };
    if !v.is_object() {
        return Fetched::Unreadable("unexpected job response".to_string());
    }

    let mut parts = Vec::new();
    for key in ["description", "requirements", "benefits"] {
        if let Some(html) = v.get(key).and_then(|val| val.as_str()) {
            let text = crate::text::html_to_text(html);
            if !text.is_empty() {
                parts.push(text);
            }
        }
    }

    let combined = parts.join("\n\n");
    let truncated = crate::text::truncate_chars(&combined, 50_000);
    if truncated.is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    let questions = form_json.map(parse_form_questions).unwrap_or_default();

    Fetched::Page(PortalPage {
        description: truncated,
        questions,
    })
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    let (account, code) = match ids(&job.url) {
        Some(pair) => pair,
        None => return crate::portals::generic::fetch(client, job).await,
    };

    let job_url = format!("https://apply.workable.com/api/v2/accounts/{account}/jobs/{code}");
    let job_body = match crate::portals::get(client, &job_url).await? {
        Http::Ok(body) => body,
        Http::Gone => return Ok(Fetched::Gone),
        Http::Blocked(code) => return Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    };

    let form_url = format!("https://apply.workable.com/api/v1/jobs/{code}/form");
    let form_body = match crate::portals::get(client, &form_url).await {
        Ok(Http::Ok(body)) => Some(body),
        _ => None,
    };

    Ok(parse(&job_body, form_body.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workable_ids() {
        assert_eq!(
            ids("https://apply.workable.com/falcomm/j/64D0E86EFE/apply"),
            Some(("falcomm".to_string(), "64D0E86EFE".to_string()))
        );
        assert_eq!(
            ids("https://apply.workable.com/tmeic-corporation-americas/j/FD4C9770FF/"),
            Some(("tmeic-corporation-americas".to_string(), "FD4C9770FF".to_string()))
        );
        assert_eq!(
            ids("https://apply.workable.com/bask-health-1/j/71AEBBCADE"),
            Some(("bask-health-1".to_string(), "71AEBBCADE".to_string()))
        );
        assert_eq!(
            ids("https://APPLY.WORKABLE.COM/falcomm/j/64D0E86EFE"),
            Some(("falcomm".to_string(), "64D0E86EFE".to_string()))
        );
        assert_eq!(
            ids("https://apply.workable.com/falcomm/j/64D0E86EFE/apply?source=linkedin#step1"),
            Some(("falcomm".to_string(), "64D0E86EFE".to_string()))
        );

        assert_eq!(ids("https://apply.workable.com/falcomm/"), None);
        assert_eq!(ids("https://apply.workable.com/j/64D0E86EFE"), None);
        assert_eq!(ids("https://example.com/falcomm/j/X"), None);
        assert_eq!(ids("https://apply.workable.com/falcomm/j/64D0E86EFE/extra/path"), None);
        assert_eq!(ids("https://apply.workable.com/falcomm/jobs/64D0E86EFE"), None);
        assert_eq!(ids("not a url"), None);
    }

    #[test]
    fn workable_parse_with_form() {
        let falcomm_job = include_str!("../../tests/fixtures/workable_falcomm.json");
        let falcomm_form = include_str!("../../tests/fixtures/workable_falcomm_form.json");
        let bask_job = include_str!("../../tests/fixtures/workable_bask.json");
        let bask_form = include_str!("../../tests/fixtures/workable_bask_form.json");

        let res = parse(falcomm_job, Some(falcomm_form));
        let page = match res {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page.description.contains("Strong Python programming skills"));
        assert!(page.description.contains("Responsibilities"));
        assert!(page.description.contains("Competitive Salary"));
        assert_eq!(page.questions.len(), 4);
        assert_eq!(
            page.questions[0],
            FormQuestion {
                label: "Are you currently located in the United States?".to_string(),
                options: vec!["Yes".to_string(), "No".to_string()],
            }
        );
        assert_eq!(
            page.questions[1],
            FormQuestion {
                label: "Are you legally eligible to work in the United States? (Proof of eligibility will be required upon employment.)".to_string(),
                options: vec!["Yes".to_string(), "No".to_string()],
            }
        );
        assert_eq!(
            page.questions[2],
            FormQuestion {
                label: "What is your availability to work during Spring 2027?".to_string(),
                options: vec!["Part-time".to_string(), "Full-time".to_string(), "Neither".to_string()],
            }
        );
        assert_eq!(
            page.questions[3],
            FormQuestion {
                label: "Are you willing to relocate or able to commute to Atlanta, GA?".to_string(),
                options: vec!["Yes".to_string(), "No".to_string()],
            }
        );

        let res_bask = parse(bask_job, Some(bask_form));
        let page_bask = match res_bask {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page_bask.description.contains("Bask Health is a software platform"));
        assert_eq!(page_bask.questions.len(), 7);
        assert!(page_bask.questions.iter().any(|q| {
            q.label == "Will you now or in the future require visa sponsorship?"
                && q.options == vec!["Yes".to_string(), "No".to_string()]
        }));
        assert!(page_bask.questions.iter().any(|q| {
            q.label == "Please upload a file containing 2–3 professional, academic, or project references, if available:"
                && q.options.is_empty()
        }));
    }

    #[test]
    fn workable_parse_edges() {
        let falcomm_job = include_str!("../../tests/fixtures/workable_falcomm.json");

        let res_none = parse(falcomm_job, None);
        let page_none = match res_none {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page_none.description.contains("Strong Python programming skills"));
        assert!(page_none.questions.is_empty());

        let res_not_found = parse(falcomm_job, Some("Not Found"));
        let page_not_found = match res_not_found {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page_not_found.description.contains("Strong Python programming skills"));
        assert!(page_not_found.questions.is_empty());

        let res_obj = parse(falcomm_job, Some("{}"));
        let page_obj = match res_obj {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page_obj.questions.is_empty());

        assert_eq!(
            parse("{}", None),
            Fetched::Unreadable("empty description".to_string())
        );

        assert_eq!(
            parse("not valid json", None),
            Fetched::Unreadable("invalid JSON".to_string())
        );

        assert_eq!(
            parse("[1, 2, 3]", None),
            Fetched::Unreadable("unexpected job response".to_string())
        );

        let single_desc = r#"{"description": "<p>A great software engineering role.</p>"}"#;
        let res_desc = parse(single_desc, None);
        let page_desc = match res_desc {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert_eq!(page_desc.description, "A great software engineering role.");
        assert!(page_desc.questions.is_empty());

        let empty_job = r#"{"description": "", "requirements": null, "benefits": "   "}"#;
        assert_eq!(
            parse(empty_job, None),
            Fetched::Unreadable("empty description".to_string())
        );
    }
}
