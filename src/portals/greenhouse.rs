#![allow(dead_code)]

use crate::model::{Fetched, FormQuestion, Job, PortalPage};
use crate::portals::Http;

pub fn api_url(job_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    if host != "job-boards.greenhouse.io" && host != "boards.greenhouse.io" {
        return None;
    }

    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() == 2 && segments[0] == "embed" && segments[1] == "job_app" {
        let mut board: Option<String> = None;
        let mut token: Option<String> = None;
        for (k, v) in url.query_pairs() {
            if k == "for" && !v.is_empty() {
                board = Some(v.into_owned());
            } else if k == "token" && !v.is_empty() {
                token = Some(v.into_owned());
            }
        }
        let b = board?;
        let t = token?;
        if !t.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(format!(
            "https://boards-api.greenhouse.io/v1/boards/{b}/jobs/{t}?questions=true"
        ));
    }

    if segments.len() == 3 && segments[1] == "jobs" {
        let board = segments[0];
        let id = segments[2];
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!(
                "https://boards-api.greenhouse.io/v1/boards/{board}/jobs/{id}?questions=true"
            ));
        }
    }

    None
}

pub fn parse(json: &str) -> anyhow::Result<PortalPage> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let raw_content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let description = if raw_content.is_empty() {
        String::new()
    } else {
        crate::text::html_to_text(&crate::text::unescape_entities(raw_content))
    };

    let mut questions = Vec::new();
    if let Some(qs) = v.get("questions").and_then(|q| q.as_array()) {
        for q in qs {
            let label = q.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string();
            let mut options = Vec::new();
            if let Some(fields) = q.get("fields").and_then(|f| f.as_array()) {
                for f in fields {
                    if let Some(values) = f.get("values").and_then(|vals| vals.as_array()) {
                        for val in values {
                            if let Some(l) = val.get("label").and_then(|l| l.as_str()) {
                                options.push(l.to_string());
                            }
                        }
                    }
                }
            }
            questions.push(FormQuestion { label, options });
        }
    }

    Ok(PortalPage {
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
        Http::Ok(json) => {
            let page = parse(&json)?;
            Ok(Fetched::Page(page))
        }
        Http::Gone => Ok(Fetched::Gone),
        Http::Blocked(code) => Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_api_url_shapes() {
        assert_eq!(
            api_url("https://job-boards.greenhouse.io/doordashusa/jobs/7848317").as_deref(),
            Some("https://boards-api.greenhouse.io/v1/boards/doordashusa/jobs/7848317?questions=true")
        );
        assert_eq!(
            api_url("https://boards.greenhouse.io/neuralink/jobs/6594261003").as_deref(),
            Some("https://boards-api.greenhouse.io/v1/boards/neuralink/jobs/6594261003?questions=true")
        );
        assert_eq!(
            api_url("https://job-boards.greenhouse.io/transmarketgroup/jobs/5151569007?gh_jid=5151569007").as_deref(),
            Some("https://boards-api.greenhouse.io/v1/boards/transmarketgroup/jobs/5151569007?questions=true")
        );
        assert_eq!(
            api_url("https://boards.greenhouse.io/embed/job_app?for=gemini&token=8214332&gh_jid=8214332").as_deref(),
            Some("https://boards-api.greenhouse.io/v1/boards/gemini/jobs/8214332?questions=true")
        );
        assert_eq!(api_url("https://boards.greenhouse.io/embed/job_app?token=7905463"), None);
        assert_eq!(api_url("https://job-boards.eu.greenhouse.io/axiomaticai/jobs/4848121101"), None);
    }

    #[test]
    fn gh_parse_sevenresearch() {
        let p = parse(include_str!("../../tests/fixtures/gh_sevenresearch.json")).unwrap();
        assert!(p.description.contains("Seven Research") && !p.description.contains("&lt;") && !p.description.contains("<p>"));
        assert!(p.questions.iter().any(|q| q.label.contains("require sponsorship")));
        assert!(p.questions.iter().any(|q| q.label.contains("GPA")));
    }

    #[test]
    fn gh_parse_doordash_options() {
        let p = parse(include_str!("../../tests/fixtures/gh_doordash.json")).unwrap();
        let q = p.questions.iter().find(|q| q.label.contains("graduation year")).unwrap();
        assert_eq!(q.options, vec!["2026","2027","2028","2029","2030","Already Graduated"]);
    }

    #[test]
    fn greenhouse_empty_content() {
        let p = parse(r#"{"content":"","questions":[]}"#).unwrap();
        assert_eq!(p.description, "");
    }
}
