#![allow(dead_code)]

use crate::model::{Fetched, FormQuestion, Job, PortalPage};
use crate::portals::Http;

pub const QUERY: &str = "query ApiJobPosting($organizationHostedJobsPageName: String!, $jobPostingId: String!) { jobPosting(organizationHostedJobsPageName: $organizationHostedJobsPageName, jobPostingId: $jobPostingId) { title descriptionHtml applicationForm { sections { fieldEntries { field } } } } }";

pub fn ids(job_url: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(job_url).ok()?;
    if url.host_str() != Some("jobs.ashbyhq.com") {
        return None;
    }
    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() == 2 {
        Some((segments[0].to_string(), segments[1].to_string()))
    } else if segments.len() == 3 && segments[2] == "application" {
        Some((segments[0].to_string(), segments[1].to_string()))
    } else {
        None
    }
}

pub fn parse(json: &str) -> anyhow::Result<Option<PortalPage>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    if let Some(errors) = v.get("errors") {
        anyhow::bail!("Ashby GraphQL error: {errors}");
    }

    let posting = match v.get("data").and_then(|d| d.get("jobPosting")) {
        Some(p) if !p.is_null() => p,
        _ => return Ok(None),
    };

    let desc_html = posting.get("descriptionHtml").and_then(|d| d.as_str()).unwrap_or("");
    let description = crate::text::html_to_text(desc_html);

    let mut questions = Vec::new();
    if let Some(sections) = posting
        .get("applicationForm")
        .and_then(|af| af.get("sections"))
        .and_then(|s| s.as_array())
    {
        for sec in sections {
            if let Some(entries) = sec.get("fieldEntries").and_then(|fe| fe.as_array()) {
                for entry in entries {
                    if let Some(field) = entry.get("field") {
                        let label = field.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let mut options = Vec::new();
                        if let Some(svs) = field.get("selectableValues").and_then(|sv| sv.as_array()) {
                            for sv in svs {
                                if let Some(opt_label) = sv.get("label").and_then(|l| l.as_str()) {
                                    options.push(opt_label.to_string());
                                }
                            }
                        }
                        questions.push(FormQuestion { label, options });
                    }
                }
            }
        }
    }

    Ok(Some(PortalPage {
        description,
        questions,
    }))
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    let (org, uuid) = match ids(&job.url) {
        Some(pair) => pair,
        None => return crate::portals::generic::fetch(client, job).await,
    };

    let url = "https://jobs.ashbyhq.com/api/non-user-graphql?op=ApiJobPosting";
    let body = serde_json::json!({
        "operationName": "ApiJobPosting",
        "variables": {
            "organizationHostedJobsPageName": org,
            "jobPostingId": uuid,
        },
        "query": QUERY,
    });

    match crate::portals::post_json(client, url, &body).await? {
        Http::Ok(json) => match parse(&json)? {
            Some(page) => Ok(Fetched::Page(page)),
            None => Ok(Fetched::Gone),
        },
        Http::Gone => Ok(Fetched::Gone),
        Http::Blocked(code) => Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ashby_ids() {
        assert_eq!(
            ids("https://jobs.ashbyhq.com/circleback/2bb6be67-d1a8-42f7-bb1b-64ee36bf613f/application?embed=true"),
            Some(("circleback".into(), "2bb6be67-d1a8-42f7-bb1b-64ee36bf613f".into()))
        );
        assert_eq!(
            ids("https://jobs.ashbyhq.com/output/da2723ca-a418-49f1-b7da-a4f383dd8239"),
            Some(("output".into(), "da2723ca-a418-49f1-b7da-a4f383dd8239".into()))
        );
        assert_eq!(ids("https://jobs.ashbyhq.com/output"), None);
    }

    #[test]
    fn ashby_parse_fields() {
        let p = parse(include_str!("../../tests/fixtures/ashby_perplexity.json")).unwrap().unwrap();
        assert!(p.questions.iter().any(|q| q.label.contains("require sponsorship")));
        let e = parse(include_str!("../../tests/fixtures/ashby_etched.json")).unwrap().unwrap();
        assert!(e.questions.iter().any(|q| q.options.contains(&"Summer (May - August)".to_string())));
    }

    #[test]
    fn ashby_missing_is_none() {
        assert!(parse(include_str!("../../tests/fixtures/ashby_missing.json")).unwrap().is_none());
    }
}
