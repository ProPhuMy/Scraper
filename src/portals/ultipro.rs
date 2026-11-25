#![allow(dead_code)]

use crate::model::{Fetched, Job, PortalPage};
use crate::portals::Http;

const MARKER: &str = "new US.Opportunity.CandidateOpportunityDetail(";

pub fn is_ultipro(job_url: &str) -> bool {
    let url = match reqwest::Url::parse(job_url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    if !(host.ends_with(".ultipro.com")
        || host == "ultipro.com"
        || host.ends_with(".ukg.net")
        || host == "ukg.net")
    {
        return false;
    }
    if !url.path().contains("/OpportunityDetail") {
        return false;
    }
    url.query_pairs()
        .any(|(k, v)| k == "opportunityId" && !v.is_empty())
}

pub fn parse(html: &str) -> Fetched {
    let Some(idx) = html.find(MARKER) else {
        return Fetched::Unreadable("no job data in page".to_string());
    };
    let rest = &html[idx + MARKER.len()..];
    let mut de = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    let val = match de.next() {
        Some(Ok(v)) => v,
        _ => return Fetched::Unreadable("invalid job json".to_string()),
    };
    let Some(obj) = val.as_object() else {
        return Fetched::Unreadable("job json is not an object".to_string());
    };

    if obj.get("OpportunityIsClosed").and_then(|v| v.as_bool()) == Some(true) {
        return Fetched::Gone;
    }

    let desc_html = match obj.get("Description").and_then(|v| v.as_str()) {
        Some(d) => d,
        None => return Fetched::Unreadable("empty description".to_string()),
    };

    let text = crate::text::html_to_text(desc_html);
    if text.trim().is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    let description = crate::text::truncate_chars(&text, 50_000);
    if description.trim().is_empty() {
        return Fetched::Unreadable("empty description".to_string());
    }

    Fetched::Page(PortalPage {
        description,
        questions: Vec::new(),
    })
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
    fn ultipro_urls() {
        assert!(is_ultipro("https://recruiting2.ultipro.com/MIL1017/JobBoard/f54234e9-dfde-b183-fd20-4fbdb19cba7a/OpportunityDetail?opportunityId=1940672b-d3b1-4e15-a834-fde16cb5a420"));
        assert!(is_ultipro("https://recruiting.ultipro.com/X/JobBoard/Y/OpportunityDetail?opportunityId=Z"));
        assert!(is_ultipro("https://btcpa.rec.pro.ukg.net/BEN1022BTLL/JobBoard/702a70c1-c1f7-4971-8626-bb99571a35a2/OpportunityDetail?opportunityId=4d519358-fb57-46be-8018-838c26e1bf4a"));
        assert!(is_ultipro("https://RECRUITING2.ULTIPRO.COM/MIL1017/JobBoard/f54234e9-dfde-b183-fd20-4fbdb19cba7a/OpportunityDetail?opportunityId=123"));
        assert!(is_ultipro("https://recruiting2.ultipro.com/MIL1017/JobBoard/abc/OpportunityDetail?utm_source=test&opportunityId=123#fragment"));

        assert!(!is_ultipro("https://example.com/JobBoard/Y/OpportunityDetail?opportunityId=Z"));
        assert!(!is_ultipro("https://recruiting.ultipro.com/X/JobBoard/Y/OpportunityDetail"));
        assert!(!is_ultipro("https://recruiting2.ultipro.com/MIL1017/JobBoard/abc/"));
        assert!(!is_ultipro("https://ultipro.com.evil.com/JobBoard/Y/OpportunityDetail?opportunityId=Z"));
        assert!(!is_ultipro("https://recruiting2.ultipro.com/MIL1017/JobBoard/abc/OpportunityDetail?opportunityId="));
        assert!(!is_ultipro("https://recruiting2.ultipro.com/MIL1017/JobBoard/abc/opportunitydetail?opportunityId=123"));
        assert!(!is_ultipro("not-a-valid-url"));
    }

    #[test]
    fn ultipro_parse_pages() {
        let milliman_html = include_str!("../../tests/fixtures/ultipro_milliman.html");
        let milliman_res = parse(milliman_html);
        match milliman_res {
            Fetched::Page(page) => {
                assert!(page.description.contains("Milliman’s Indianapolis Health practice"));
                assert!(page.questions.is_empty());
            }
            other => panic!("expected Fetched::Page for milliman, got {:?}", other),
        }

        let ukg_html = include_str!("../../tests/fixtures/ultipro_ukg.html");
        let ukg_res = parse(ukg_html);
        match ukg_res {
            Fetched::Page(page) => {
                assert!(!page.description.is_empty());
                assert!(page.description.contains("Bennett Thrasher"));
                assert!(page.questions.is_empty());
            }
            other => panic!("expected Fetched::Page for ukg, got {:?}", other),
        }
    }

    #[test]
    fn ultipro_parse_closed_and_junk() {
        let milliman_html = include_str!("../../tests/fixtures/ultipro_milliman.html");
        let closed_html = milliman_html.replace("\"OpportunityIsClosed\":false", "\"OpportunityIsClosed\":true");
        assert_eq!(parse(&closed_html), Fetched::Gone);

        let missing_html = include_str!("../../tests/fixtures/ultipro_missing.html");
        assert_eq!(parse(missing_html), Fetched::Unreadable("no job data in page".into()));

        assert_eq!(parse("<html></html>"), Fetched::Unreadable("no job data in page".into()));

        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail({\"Id\":"),
            Fetched::Unreadable("invalid job json".into())
        );

        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail({\"Description\":\"\"})"),
            Fetched::Unreadable("empty description".into())
        );
        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail({\"Description\":null})"),
            Fetched::Unreadable("empty description".into())
        );
        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail({})"),
            Fetched::Unreadable("empty description".into())
        );
        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail({\"Description\":\"<p>  \\n </p>\"})"),
            Fetched::Unreadable("empty description".into())
        );

        assert_eq!(
            parse("new US.Opportunity.CandidateOpportunityDetail([1, 2])"),
            Fetched::Unreadable("job json is not an object".into())
        );

        let non_bool_closed = "new US.Opportunity.CandidateOpportunityDetail({\"Description\":\"<p>Active Job</p>\",\"OpportunityIsClosed\":\"not_a_bool\"})";
        match parse(non_bool_closed) {
            Fetched::Page(page) => assert_eq!(page.description, "Active Job"),
            other => panic!("expected Fetched::Page, got {:?}", other),
        }

        let multi_markers = "new US.Opportunity.CandidateOpportunityDetail({\"OpportunityIsClosed\":true}) new US.Opportunity.CandidateOpportunityDetail({\"OpportunityIsClosed\":false,\"Description\":\"<p>Active</p>\"})";
        assert_eq!(parse(multi_markers), Fetched::Gone);

        let huge_content = "x".repeat(60_000);
        let huge_desc_json = format!("new US.Opportunity.CandidateOpportunityDetail({{\"Description\":\"<p>{huge_content}</p>\"}})");
        match parse(&huge_desc_json) {
            Fetched::Page(page) => {
                assert_eq!(page.description.chars().count(), 50_000);
            }
            other => panic!("expected Fetched::Page, got {:?}", other),
        }
    }
}
