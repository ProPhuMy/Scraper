#![allow(dead_code)]

use crate::model::{Fetched, Job, PortalPage};
use crate::portals::Http;

pub fn parse(html: &str) -> Fetched {
    let text = crate::text::html_to_text(html);
    let count = text.chars().count();
    let n = crate::text::normalize(&text);

    if count < 500 {
        return Fetched::Unreadable("page too short; probably needs JavaScript".to_string());
    }

    if count < 2000
        && (n.contains("enable javascript")
            || n.contains("requires javascript")
            || n.contains("javascript is disabled"))
    {
        return Fetched::Unreadable("page too short; probably needs JavaScript".to_string());
    }

    Fetched::Page(PortalPage {
        description: crate::text::truncate_chars(&text, 50_000),
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
}
