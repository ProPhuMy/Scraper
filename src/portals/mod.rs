#![allow(dead_code)]

pub mod ashby;
pub mod generic;
pub mod greenhouse;
pub mod lever;
pub mod bamboohr;
pub mod oracle;
pub mod ultipro;
pub mod workable;
pub mod workday;

use std::time::Duration;
use crate::model::{Fetched, Job, Provider};

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

pub fn client() -> anyhow::Result<reqwest::Client> {
    let c = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()?;
    Ok(c)
}

#[derive(Debug, PartialEq)]
pub enum Http {
    Ok(String),
    Gone,
    Blocked(u16),
}

enum Method<'a> {
    Get,
    PostJson(&'a serde_json::Value),
}

async fn send_with_retry(client: &reqwest::Client, url: &str, method: Method<'_>) -> anyhow::Result<Http> {
    let mut last_err = None;
    for attempt in 1..=3 {
        let req = match method {
            Method::Get => client.get(url),
            Method::PostJson(body) => client.post(url).json(body),
        };
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
                    return Ok(Http::Gone);
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    if attempt < 3 {
                        let header = resp.headers().get("Retry-After").and_then(|h| h.to_str().ok());
                        let sleep_secs = std::cmp::max(retry_after_secs(header), 1 << attempt);
                        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                        continue;
                    } else {
                        anyhow::bail!("HTTP {} after 3 attempts", status);
                    }
                }
                if status.is_client_error() {
                    return Ok(Http::Blocked(status.as_u16()));
                }
                let body = resp.text().await?;
                return Ok(Http::Ok(body));
            }
            Err(e) => {
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    last_err = Some(e);
                } else {
                    return Err(anyhow::anyhow!(e));
                }
            }
        }
    }
    if let Some(e) = last_err {
        Err(anyhow::anyhow!(e))
    } else {
        anyhow::bail!("Request failed after 3 attempts")
    }
}

pub async fn get(client: &reqwest::Client, url: &str) -> anyhow::Result<Http> {
    send_with_retry(client, url, Method::Get).await
}

pub async fn post_json(client: &reqwest::Client, url: &str, body: &serde_json::Value) -> anyhow::Result<Http> {
    send_with_retry(client, url, Method::PostJson(body)).await
}

pub fn retry_after_secs(header: Option<&str>) -> u64 {
    match header {
        Some(s) => match s.trim().parse::<u64>() {
            Ok(secs) => secs.min(30),
            Err(_) => 0,
        },
        None => 0,
    }
}

pub async fn fetch(client: &reqwest::Client, job: &Job) -> anyhow::Result<Fetched> {
    match job.provider {
        Provider::Greenhouse => {
            if greenhouse::api_url(&job.url).is_none() {
                generic::fetch(client, job).await
            } else {
                greenhouse::fetch(client, job).await
            }
        }
        Provider::Ashby => {
            if ashby::ids(&job.url).is_none() {
                generic::fetch(client, job).await
            } else {
                ashby::fetch(client, job).await
            }
        }
        Provider::Workday => {
            if workday::cxs_url(&job.url).is_none() {
                generic::fetch(client, job).await
            } else {
                workday::fetch(client, job).await
            }
        }
        Provider::Lever => lever::fetch(client, job).await,
        Provider::Other => {
            if oracle::api_url(&job.url).is_some() {
                oracle::fetch(client, job).await
            } else if ultipro::is_ultipro(&job.url) {
                ultipro::fetch(client, job).await
            } else if bamboohr::api_url(&job.url).is_some() {
                bamboohr::fetch(client, job).await
            } else if workable::ids(&job.url).is_some() {
                workable::fetch(client, job).await
            } else {
                generic::fetch(client, job).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_parsing() {
        assert_eq!(retry_after_secs(Some("5")), 5);
        assert_eq!(retry_after_secs(Some("999")), 30);
        assert_eq!(retry_after_secs(Some("Wed, 21 Oct 2026 07:28:00 GMT")), 0);
        assert_eq!(retry_after_secs(None), 0);
    }

    #[test]
    #[ignore]
    fn live_greenhouse() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = client().unwrap();
            let job = Job {
                id: 1,
                simplify_id: "test".into(),
                company: "DoorDash".into(),
                title: "AI Research Fellowship".into(),
                url: "https://job-boards.greenhouse.io/doordashusa/jobs/7848317".into(),
                provider: Provider::Greenhouse,
            };
            let res = greenhouse::fetch(&client, &job).await.unwrap();
            if let Fetched::Page(page) = res {
                assert!(page.questions.len() > 5);
            } else {
                panic!("Expected Fetched::Page");
            }
        });
    }
}
