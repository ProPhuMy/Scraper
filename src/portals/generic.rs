#![allow(dead_code)]

use crate::model::{Fetched, Job, PortalPage};
use crate::portals::Http;

pub fn parse(html: &str) -> Fetched {
    let text = crate::text::html_to_text(html);
    let count = text.chars().count();
    let n = crate::text::normalize(&text);

    if count < 500 {
        if let Some(desc) = json_ld_description(html) {
            return Fetched::Page(PortalPage {
                description: desc,
                questions: Vec::new(),
            });
        }
        return Fetched::Unreadable("page too short; probably needs JavaScript".to_string());
    }

    if count < 2000
        && (n.contains("enable javascript")
            || n.contains("requires javascript")
            || n.contains("javascript is disabled"))
    {
        if let Some(desc) = json_ld_description(html) {
            return Fetched::Page(PortalPage {
                description: desc,
                questions: Vec::new(),
            });
        }
        return Fetched::Unreadable("page too short; probably needs JavaScript".to_string());
    }

    Fetched::Page(PortalPage {
        description: crate::text::truncate_chars(&text, 50_000),
        questions: Vec::new(),
    })
}

fn is_job_posting_type(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    match map.get("@type") {
        Some(serde_json::Value::String(s)) => s == "JobPosting",
        Some(serde_json::Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some("JobPosting")),
        _ => false,
    }
}

fn check_candidate_object(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if !is_job_posting_type(map) {
        return None;
    }

    let desc_raw = map.get("description").and_then(|v| v.as_str())?;
    if desc_raw.trim().is_empty() {
        return None;
    }

    let mut desc_text = crate::text::html_to_text(desc_raw);
    let lower = desc_text.to_ascii_lowercase();
    if lower.contains("<p") || lower.contains("<br") || lower.contains("<li") {
        desc_text = crate::text::html_to_text(&desc_text);
    }
    if desc_text.trim().is_empty() {
        return None;
    }

    let title = map
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let full_text = match title {
        Some(t) => format!("{t}\n\n{desc_text}"),
        None => desc_text,
    };

    Some(crate::text::truncate_chars(&full_text, 50_000))
}

fn extract_graph_job_posting(graph: &serde_json::Value) -> Option<String> {
    match graph {
        serde_json::Value::Array(items) => {
            for item in items {
                if let serde_json::Value::Object(map) = item {
                    if let Some(text) = check_candidate_object(map) {
                        return Some(text);
                    }
                }
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(text) = check_candidate_object(map) {
                return Some(text);
            }
        }
        _ => {}
    }
    None
}

fn extract_job_posting_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(text) = check_candidate_object(map) {
                return Some(text);
            }
            if let Some(graph) = map.get("@graph") {
                if let Some(text) = extract_graph_job_posting(graph) {
                    return Some(text);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let serde_json::Value::Object(map) = item {
                    if let Some(text) = check_candidate_object(map) {
                        return Some(text);
                    }
                    if let Some(graph) = map.get("@graph") {
                        if let Some(text) = extract_graph_job_posting(graph) {
                            return Some(text);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    None
}

pub fn json_ld_description(html: &str) -> Option<String> {
    let doc = scraper::Html::parse_document(html);
    let Ok(selector) = scraper::Selector::parse(r#"script[type="application/ld+json"]"#) else {
        return None;
    };

    for element in doc.select(&selector) {
        let script_text = element.text().collect::<String>();
        let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&script_text) else {
            continue;
        };

        if let Some(result) = extract_job_posting_text(&json_val) {
            return Some(result);
        }
    }

    None
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    match crate::portals::get(client, &job.url).await? {
        Http::Ok(html) => Ok(parse(&html)),
        Http::Gone => Ok(Fetched::Gone),
        Http::Blocked(code) => Ok(Fetched::Unreadable(format!("HTTP {code}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_js_shell_is_unreadable() {
        assert!(matches!(
            parse("<html><body><noscript>You need to enable JavaScript to run this app.</noscript><div id=root></div></body></html>"),
            Fetched::Unreadable(_)
        ));
    }

    #[test]
    fn generic_real_page_is_page() {
        let body = format!(
            "<html><body><h1>Intern</h1><p>{}</p></body></html>",
            "We build things. ".repeat(60)
        );
        assert!(matches!(parse(&body), Fetched::Page(_)));
    }

    #[test]
    fn jsonld_docusign() {
        let html = include_str!("../../tests/fixtures/jsonld_docusign.html");
        match parse(html) {
            Fetched::Page(page) => {
                assert!(page.description.contains("Product Data Science Intern"));
                assert!(page.description.contains("Docusign brings agreements to life"));
                assert!(page.description.contains("simplify people’s lives"));
                assert!(page.questions.is_empty());
            }
            other => panic!("expected Fetched::Page, got {other:?}"),
        }
    }

    #[test]
    fn jsonld_jibe() {
        let html = include_str!("../../tests/fixtures/jsonld_jibe.html");
        match parse(html) {
            Fetched::Page(page) => {
                assert!(page.description.contains("Data Engineering Intern - Summer 2027"));
                assert!(page.description.contains("Systems Planning and Analysis, Inc. (SPA)"));
                assert!(page.questions.is_empty());
            }
            other => panic!("expected Fetched::Page, got {other:?}"),
        }
    }

    #[test]
    fn jsonld_shapes() {
        // 1. JobPosting inside a top-level array -> found
        let html_array = r#"<html><head><script type="application/ld+json">
            [{"@type": "Organization", "name": "Acme"}, {"@type": "JobPosting", "title": "Array Job", "description": "<p>Array desc</p>"}]
        </script></head><body></body></html>"#;
        assert_eq!(
            json_ld_description(html_array).as_deref(),
            Some("Array Job\n\nArray desc")
        );

        // 2. JobPosting inside @graph -> found
        let html_graph = r#"<html><head><script type="application/ld+json">
            {"@context": "https://schema.org", "@graph": [{"@type": "JobPosting", "title": "Graph Job", "description": "<p>Graph desc</p>"}]}
        </script></head><body></body></html>"#;
        assert_eq!(
            json_ld_description(html_graph).as_deref(),
            Some("Graph Job\n\nGraph desc")
        );

        // 3. @type as array ["JobPosting"] -> found
        let html_type_array = r#"<html><head><script type="application/ld+json">
            {"@type": ["JobPosting", "SpecialJob"], "title": "MultiType Job", "description": "<p>Multi desc</p>"}
        </script></head><body></body></html>"#;
        assert_eq!(
            json_ld_description(html_type_array).as_deref(),
            Some("MultiType Job\n\nMulti desc")
        );

        // 4. Only an Organization -> None
        let html_org = r#"<html><head><script type="application/ld+json">
            {"@type": "Organization", "name": "Acme Corp"}
        </script></head><body></body></html>"#;
        assert_eq!(json_ld_description(html_org), None);

        // 5. Broken JSON in first script, valid JobPosting in second -> found
        let html_broken_first = r#"<html><head>
            <script type="application/ld+json">{"@type": "JobPosting", broken,</script>
            <script type="application/ld+json">{"@type": "JobPosting", "title": "Second Script", "description": "<p>Second desc</p>"}</script>
        </head><body></body></html>"#;
        assert_eq!(
            json_ld_description(html_broken_first).as_deref(),
            Some("Second Script\n\nSecond desc")
        );

        // 6. JobPosting with empty description -> None
        let html_empty_desc = r#"<html><head><script type="application/ld+json">
            {"@type": "JobPosting", "title": "No Desc", "description": ""}
        </script></head><body></body></html>"#;
        assert_eq!(json_ld_description(html_empty_desc), None);
    }

    #[test]
    fn jsonld_double_escaped_html() {
        let html = r#"<html><head><script type="application/ld+json">
            {"@type": "JobPosting", "title": "Escaped Job", "description": "&lt;p&gt;Double escaped paragraph&lt;/p&gt;&lt;ul&gt;&lt;li&gt;Item 1&lt;/li&gt;&lt;/ul&gt;"}
        </script></head><body></body></html>"#;
        let desc = json_ld_description(html).expect("must parse double escaped html");
        assert!(desc.contains("Escaped Job\n\nDouble escaped paragraph\n- Item 1"));
        assert!(!desc.contains("<p"));
        assert!(!desc.contains("<li"));
    }

    #[test]
    fn jsonld_no_title() {
        let html = r#"<html><head><script type="application/ld+json">
            {"@type": "JobPosting", "description": "<p>Only description here</p>"}
        </script></head><body></body></html>"#;
        assert_eq!(
            json_ld_description(html).as_deref(),
            Some("Only description here")
        );
    }
}
