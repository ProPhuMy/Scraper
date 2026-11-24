#![allow(dead_code)]

use crate::model::{Fetched, Job};

pub async fn fetch(_client: &reqwest::Client, _job: &Job) -> anyhow::Result<Fetched> {
    Ok(Fetched::Unreadable("not implemented".into()))
}
