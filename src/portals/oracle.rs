#![allow(dead_code)]

use crate::model::{Fetched, Job, PortalPage};
use crate::portals::Http;

pub fn api_url(job_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(job_url).ok()?;
    let host = url.host_str()?;
    if !host.ends_with(".oraclecloud.com") || host.len() <= ".oraclecloud.com".len() {
        return None;
    }

    let path = url.path().trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() < 7 {
        return None;
    }

    if segments[0] != "hcmUI" || segments[1] != "CandidateExperience" {
        return None;
    }

    let locale = segments[2];
    if locale.is_empty() {
        return None;
    }

    if segments[3] != "sites" {
        return None;
    }

    let site = segments[4];
    if site.is_empty() || !site.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }

    if segments[5] != "job" {
        return None;
    }

    let id = segments[6];
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    Some(format!(
        "https://{host}/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?expand=all&onlyData=true&finder=ById;Id=%22{id}%22,siteNumber={site}"
    ))
}

pub fn parse(body: &str) -> Fetched {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Fetched::Unreadable(format!("invalid JSON: {e}")),
    };

    let items = match v.get("items").and_then(|i| i.as_array()) {
        Some(arr) => arr,
        None => return Fetched::Unreadable("missing or invalid items array".to_string()),
    };

    if items.is_empty() {
        return Fetched::Gone;
    }

    let item = match items.first().and_then(|i| i.as_object()) {
        Some(obj) => obj,
        None => return Fetched::Unreadable("items[0] is not an object".to_string()),
    };

    let desc_fields = [
        "ShortDescriptionStr",
        "ExternalDescriptionStr",
        "ExternalResponsibilitiesStr",
        "ExternalQualificationsStr",
    ];
    let mut parts: Vec<String> = Vec::new();

    for field in desc_fields {
        if let Some(val) = item.get(field).and_then(|v| v.as_str()) {
            if !val.trim().is_empty() {
                let text = crate::text::html_to_text(val);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
    }

    let mut flex_lines: Vec<String> = Vec::new();
    if let Some(flex_fields) = item.get("requisitionFlexFields").and_then(|f| f.as_array()) {
        for ff in flex_fields {
            let prompt = ff
                .get("Prompt")
                .and_then(|p| p.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let value = ff.get("Value").and_then(|v| {
                if let Some(s) = v.as_str() {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                } else if let Some(n) = v.as_number() {
                    Some(n.to_string())
                } else {
                    None
                }
            });

            if let (Some(p), Some(val)) = (prompt, value) {
                flex_lines.push(format!("{p}: {val}"));
            }
        }
    }

    if !flex_lines.is_empty() {
        parts.push(flex_lines.join("\n"));
    }

    if parts.is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    let description = parts.join("\n\n");
    let description = crate::text::truncate_chars(&description, 50_000);
    if description.is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    Fetched::Page(PortalPage {
        description,
        questions: Vec::new(),
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
    fn oracle_api_url_shapes() {
        assert_eq!(
            api_url("https://hdhl.fa.us6.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1/job/1009005"),
            Some("https://hdhl.fa.us6.oraclecloud.com/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?expand=all&onlyData=true&finder=ById;Id=%221009005%22,siteNumber=CX_1".to_string())
        );
        assert_eq!(
            api_url("https://egup.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX/job/20286620/?utm_source=x"),
            Some("https://egup.fa.us2.oraclecloud.com/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?expand=all&onlyData=true&finder=ById;Id=%2220286620%22,siteNumber=CX".to_string())
        );
        assert_eq!(
            api_url("https://x.fa.em2.oraclecloud.com/hcmUI/CandidateExperience/fr-FR/sites/Honeywell/job/42"),
            Some("https://x.fa.em2.oraclecloud.com/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?expand=all&onlyData=true&finder=ById;Id=%2242%22,siteNumber=Honeywell".to_string())
        );
        assert_eq!(
            api_url("https://ibwujb.fa.ocs.oraclecloud.com/hcmUI/CandidateExperience/en-US/sites/ULSolutionsCareers/job/12345/extra/path?foo=bar#frag"),
            Some("https://ibwujb.fa.ocs.oraclecloud.com/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?expand=all&onlyData=true&finder=ById;Id=%2212345%22,siteNumber=ULSolutionsCareers".to_string())
        );
        assert_eq!(api_url("https://example.com/hcmUI/CandidateExperience/en/sites/CX/job/1"), None);
        assert_eq!(api_url("https://oraclecloud.com.evil.com/hcmUI/CandidateExperience/en/sites/CX/job/1"), None);
        assert_eq!(api_url("https://hdhl.fa.us6.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1/requisitions"), None);
        assert_eq!(api_url("https://hdhl.fa.us6.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1/job/abc"), None);
        assert_eq!(api_url("https://hdhl.fa.us6.oraclecloud.com/hcmUI/CandidateExperience/en/sites/invalid site!/job/1009005"), None);
    }

    #[test]
    fn oracle_parse_pages() {
        let stantec = parse(include_str!("../../tests/fixtures/oracle_stantec.json"));
        match stantec {
            Fetched::Page(page) => {
                assert!(page.description.contains("there’s no better place"));
                assert!(page.description.contains("Minimum Experience (Total Years): 0"));
                assert_eq!(page.questions, Vec::new());
            }
            other => panic!("expected Fetched::Page, got {other:?}"),
        }

        let tektronix = parse(include_str!("../../tests/fixtures/oracle_tektronix.json"));
        match tektronix {
            Fetched::Page(page) => {
                assert!(page.description.contains("Tektronix is seeking a 12-week Software Engineering Intern"));
                assert_eq!(page.questions, Vec::new());
            }
            other => panic!("expected Fetched::Page, got {other:?}"),
        }

        let vertiv = parse(include_str!("../../tests/fixtures/oracle_vertiv.json"));
        match vertiv {
            Fetched::Page(page) => {
                assert!(page.description.contains("MindOps"));
                assert_eq!(page.questions, Vec::new());
            }
            other => panic!("expected Fetched::Page, got {other:?}"),
        }
    }

    #[test]
    fn oracle_parse_closed_and_junk() {
        assert_eq!(
            parse(include_str!("../../tests/fixtures/oracle_closed.json")),
            Fetched::Gone
        );
        assert!(matches!(parse("{}"), Fetched::Unreadable(_)));
        assert!(matches!(parse("not json"), Fetched::Unreadable(_)));
        assert!(matches!(
            parse(r#"{"items":[{"ExternalDescriptionStr":null}]}"#),
            Fetched::Unreadable(_)
        ));
    }

    #[test]
    fn oracle_parse_flex_fields_only() {
        let json = r#"{
            "items": [{
                "ExternalDescriptionStr": "",
                "requisitionFlexFields": [
                    {"Prompt": "Staff Type Category", "Value": "HL - Hourly Limited Benefits"},
                    {"Prompt": "Empty Val", "Value": ""},
                    {"Prompt": "Null Val", "Value": null},
                    {"Prompt": "", "Value": "Ignored"}
                ]
            }]
        }"#;
        match parse(json) {
            Fetched::Page(page) => {
                assert_eq!(page.description, "Staff Type Category: HL - Hourly Limited Benefits");
                assert!(page.questions.is_empty());
            }
            other => panic!("expected Page, got {other:?}"),
        }
    }

    #[test]
    fn oracle_parse_missing_or_invalid_items() {
        assert!(matches!(parse(r#"{"items": "not an array"}"#), Fetched::Unreadable(_)));
        assert!(matches!(parse(r#"{"items": [42]}"#), Fetched::Unreadable(_)));
        assert!(matches!(parse(r#"{"items": [{}]}"#), Fetched::Unreadable(_)));
        assert!(matches!(parse(r#"{"items": [{"ExternalDescriptionStr": ""}]}"#), Fetched::Unreadable(_)));
    }
}
