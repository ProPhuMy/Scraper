#![allow(dead_code)]

use crate::model::{Fetched, FormQuestion, Job, PortalPage};
use crate::portals::Http;

pub fn api_url(job_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    if !host.ends_with(".bamboohr.com") || host.len() <= ".bamboohr.com".len() {
        return None;
    }

    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() >= 2 && segments[0] == "careers" {
        let id = segments[1];
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("https://{host}/careers/{id}/detail"));
        }
    }

    None
}

fn parse_option(opt: &serde_json::Value) -> Option<String> {
    if let Some(s) = opt.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    } else if let Some(obj) = opt.as_object() {
        let opt_str = obj
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                obj.get("value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                obj.get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
            });
        if let Some(s) = opt_str {
            return Some(s.to_string());
        }
    }
    None
}

pub fn parse(body: &str) -> Fetched {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Fetched::Unreadable(format!("invalid JSON: {e}")),
    };

    let result = match v.get("result") {
        Some(r) if r.is_object() => r,
        _ => return Fetched::Unreadable("missing or invalid result".to_string()),
    };

    let job_opening = match result.get("jobOpening") {
        Some(j) if j.is_object() => j,
        _ => return Fetched::Unreadable("missing or invalid jobOpening".to_string()),
    };

    let desc_html = match job_opening.get("description").and_then(|d| d.as_str()) {
        Some(d) => d,
        None => return Fetched::Unreadable("missing or invalid description".to_string()),
    };

    let desc_text = crate::text::html_to_text(desc_html);
    let description = crate::text::truncate_chars(&desc_text, 50_000);
    if description.is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    let mut questions = Vec::new();
    if let Some(cqs) = result
        .get("formFields")
        .and_then(|ff| ff.get("customQuestions"))
        .and_then(|cq| cq.as_array())
    {
        for item in cqs {
            let label = match item.get("question").and_then(|q| q.as_str()) {
                Some(q) => {
                    let trimmed = q.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    trimmed.to_string()
                }
                None => continue,
            };

            let mut options = Vec::new();
            if let Some(opts) = item.get("options").and_then(|o| o.as_array()) {
                for opt in opts {
                    if let Some(opt_val) = parse_option(opt) {
                        options.push(opt_val);
                    }
                }
            }

            questions.push(FormQuestion { label, options });
        }
    }

    Fetched::Page(PortalPage {
        description,
        questions,
    })
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    let url = match api_url(&job.url) {
        Some(u) => u,
        None => return crate::portals::generic::fetch(client, job).await,
    };
    match crate::portals::get(client, &url).await? {
        Http::Ok(body) => Ok(parse(&body)),
        Http::Gone => Ok(Fetched::Gone),
        Http::Blocked(code) => Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bamboohr_api_url() {
        assert_eq!(
            api_url("https://silverlogic.bamboohr.com/careers/36/"),
            Some("https://silverlogic.bamboohr.com/careers/36/detail".to_string())
        );
        assert_eq!(
            api_url("https://smule.bamboohr.com/careers/31"),
            Some("https://smule.bamboohr.com/careers/31/detail".to_string())
        );
        assert_eq!(
            api_url("https://smule.bamboohr.com/careers/31?source=x"),
            Some("https://smule.bamboohr.com/careers/31/detail".to_string())
        );
        assert_eq!(
            api_url("https://smule.bamboohr.com/careers/31/detail"),
            Some("https://smule.bamboohr.com/careers/31/detail".to_string())
        );
        assert_eq!(
            api_url("https://SMULE.BambooHR.COM/careers/31"),
            Some("https://smule.bamboohr.com/careers/31/detail".to_string())
        );
        assert_eq!(api_url("https://smule.bamboohr.com/careers/"), None);
        assert_eq!(api_url("https://smule.bamboohr.com/jobs/view.php?id=3"), None);
        assert_eq!(api_url("https://bamboohr.com.evil.com/careers/3"), None);
        assert_eq!(api_url("https://evilbamboohr.com/careers/3"), None);
        assert_eq!(api_url("https://bamboohr.com/careers/3"), None);
        assert_eq!(api_url("https://smule.bamboohr.com/careers/abc"), None);
        assert_eq!(api_url("not a url"), None);
    }

    #[test]
    fn bamboohr_parse_pages() {
        // smule fixture
        let smule = include_str!("../../tests/fixtures/bamboohr_smule.json");
        let smule_res = parse(smule);
        let smule_page = match smule_res {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page for smule, got {other:?}"),
        };
        assert!(smule_page.description.contains("Smule has been on a mission"));
        assert_eq!(smule_page.questions.len(), 3);
        assert!(smule_page
            .questions
            .iter()
            .any(|q| q.label.contains("require employer sponsorship")));
        for q in &smule_page.questions {
            assert!(q.options.is_empty());
        }

        // silverlogic fixture
        let silverlogic = include_str!("../../tests/fixtures/bamboohr_silverlogic.json");
        let silverlogic_res = parse(silverlogic);
        let silverlogic_page = match silverlogic_res {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page for silverlogic, got {other:?}"),
        };
        assert!(silverlogic_page.description.contains("BRDGIT LLC"));
        assert!(silverlogic_page.questions.len() >= 2);
        assert!(silverlogic_page
            .questions
            .iter()
            .any(|q| q.label.contains("Boca Raton office")));
    }

    #[test]
    fn bamboohr_parse_junk() {
        assert!(matches!(parse("{}"), Fetched::Unreadable(_)));
        assert!(matches!(parse(""), Fetched::Unreadable(_)));
        assert!(matches!(parse("not json"), Fetched::Unreadable(_)));
        assert!(matches!(parse(r#"{"result":null}"#), Fetched::Unreadable(_)));
        assert!(matches!(
            parse(r#"{"result":{"jobOpening":null}}"#),
            Fetched::Unreadable(_)
        ));

        // Valid jobOpening without formFields returns Page with empty questions
        let json_no_form = r#"{"result":{"jobOpening":{"description":"<p>Hello world ...</p>"}}}"#;
        let page_no_form = match parse(json_no_form) {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert_eq!(page_no_form.description, "Hello world ...");
        assert!(page_no_form.questions.is_empty());

        // customQuestions with string options, object options with text, and empty question skipping
        let json_options = r#"{
            "result": {
                "jobOpening": { "description": "<p>Description</p>" },
                "formFields": {
                    "customQuestions": [
                        { "question": "Pick", "options": [{ "id": "1", "text": "A" }, "B"] },
                        { "question": "", "options": ["C"] },
                        { "question": "   ", "options": ["D"] }
                    ]
                }
            }
        }"#;
        let page_options = match parse(json_options) {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert_eq!(page_options.questions.len(), 1);
        assert_eq!(page_options.questions[0].label, "Pick");
        assert_eq!(
            page_options.questions[0].options,
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn bamboohr_parse_options_and_edge_cases() {
        let json = r#"{
            "result": {
                "jobOpening": { "description": "<p>Description</p>" },
                "formFields": {
                    "customQuestions": [
                        {
                            "question": "Question 1",
                            "options": [
                                { "text": "From text", "value": "From value" },
                                { "value": "From value only" },
                                { "label": "From label only" },
                                { "text": "" },
                                { "other": "ignored" },
                                null,
                                42,
                                "",
                                "   ",
                                "  Trimmed string  "
                            ]
                        }
                    ]
                }
            }
        }"#;
        let page = match parse(json) {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert_eq!(page.questions.len(), 1);
        assert_eq!(
            page.questions[0].options,
            vec![
                "From text".to_string(),
                "From value only".to_string(),
                "From label only".to_string(),
                "Trimmed string".to_string()
            ]
        );
    }

    #[test]
    fn bamboohr_empty_description_is_unreadable() {
        let json = r#"{"result":{"jobOpening":{"description":"<p>   <br/></p>"}}}"#;
        assert!(matches!(parse(json), Fetched::Unreadable(_)));
    }

    #[test]
    fn bamboohr_non_ascii_preserved() {
        let json = r#"{
            "result": {
                "jobOpening": { "description": "<p>curly ’ and en dash – and résumé</p>" },
                "formFields": {
                    "customQuestions": [
                        { "question": "Do you need sponsorship for café’s visa?" }
                    ]
                }
            }
        }"#;
        let page = match parse(json) {
            Fetched::Page(p) => p,
            other => panic!("expected Fetched::Page, got {other:?}"),
        };
        assert!(page.description.contains("curly ’ and en dash – and résumé"));
        assert_eq!(
            page.questions[0].label,
            "Do you need sponsorship for café’s visa?"
        );
    }
}
