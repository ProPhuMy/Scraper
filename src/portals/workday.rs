#![allow(dead_code)]

use crate::model::{Fetched, Job, PortalPage};
use crate::portals::Http;

pub fn cxs_url(job_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    if !host.ends_with(".myworkdayjobs.com") {
        return None;
    }
    let tenant = host.split('.').next()?;
    if tenant.is_empty() {
        return None;
    }

    let path = url.path().trim_matches('/');
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let locale_re = regex::Regex::new(r"^[a-z]{2}-[A-Z]{2}$").ok()?;
    if !segments.is_empty() && locale_re.is_match(segments[0]) {
        segments.remove(0);
    }

    if segments.len() >= 3 && segments[1] == "job" {
        let site = segments[0];
        let rest = segments[2..].join("/");
        Some(format!("https://{host}/wday/cxs/{tenant}/{site}/job/{rest}"))
    } else {
        None
    }
}

pub fn parse(json: &str) -> anyhow::Result<Option<PortalPage>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let info = match v.get("jobPostingInfo") {
        Some(i) if !i.is_null() => i,
        _ => return Ok(None),
    };

    let desc_html = info.get("jobDescription").and_then(|d| d.as_str()).unwrap_or("");
    let description = crate::text::html_to_text(desc_html);

    Ok(Some(PortalPage {
        description,
        questions: Vec::new(),
    }))
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    let url = match cxs_url(&job.url) {
        Some(u) => u,
        None => return crate::portals::generic::fetch(client, job).await,
    };
    match crate::portals::get(client, &url).await? {
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
    fn workday_cxs_shapes() {
        assert_eq!(
            cxs_url("https://centific.wd1.myworkdayjobs.com/Centific_Global/job/Remote-Work-USA/Speech-Intern-4_JR107037").as_deref(),
            Some("https://centific.wd1.myworkdayjobs.com/wday/cxs/centific/Centific_Global/job/Remote-Work-USA/Speech-Intern-4_JR107037")
        );
        assert_eq!(
            cxs_url("https://clio.wd3.myworkdayjobs.com/en-US/ClioCareerSite/job/Toronto/Software-Developer--Co-op_REQ-1577").as_deref(),
            Some("https://clio.wd3.myworkdayjobs.com/wday/cxs/clio/ClioCareerSite/job/Toronto/Software-Developer--Co-op_REQ-1577")
        );
        assert_eq!(
            cxs_url("https://gevernova.wd5.myworkdayjobs.com/vernova_externalsite/job/Software-Engineering-Intern_R5057006-2").as_deref(),
            Some("https://gevernova.wd5.myworkdayjobs.com/wday/cxs/gevernova/vernova_externalsite/job/Software-Engineering-Intern_R5057006-2")
        );
        assert_eq!(
            cxs_url("https://pg.wd5.myworkdayjobs.com/1000/job/CINCINNATI/X_R1").as_deref(),
            Some("https://pg.wd5.myworkdayjobs.com/wday/cxs/pg/1000/job/CINCINNATI/X_R1")
        );
        assert_eq!(cxs_url("https://example.com/a/job/b"), None);
    }

    #[test]
    fn workday_parse() {
        let p = parse(include_str!("../../tests/fixtures/workday_centific.json")).unwrap().unwrap();
        assert!(p.description.contains("Centific") && p.questions.is_empty());
        assert!(parse(r#"{"error":"x"}"#).unwrap().is_none());
    }
}
