#![allow(dead_code)]

use crate::model::{NewJob, Provider};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

const US_STATE_CODES: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA",
    "HI", "ID", "IL", "IN", "IA", "KS", "KY", "LA", "ME", "MD",
    "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ",
    "NM", "NY", "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC",
    "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV", "WI", "WY",
    "DC",
];

const US_STATE_NAMES: &[&str] = &[
    "alabama", "alaska", "arizona", "arkansas", "california", "colorado",
    "connecticut", "delaware", "florida", "georgia", "hawaii", "idaho",
    "illinois", "indiana", "iowa", "kansas", "kentucky", "louisiana",
    "maine", "maryland", "massachusetts", "michigan", "minnesota",
    "mississippi", "missouri", "montana", "nebraska", "nevada",
    "new hampshire", "new jersey", "new mexico", "new york",
    "north carolina", "north dakota", "ohio", "oklahoma", "oregon",
    "pennsylvania", "rhode island", "south carolina", "south dakota",
    "tennessee", "texas", "utah", "vermont", "virginia", "washington",
    "west virginia", "wisconsin", "wyoming", "district of columbia",
];

static RELEVANT_TITLE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(software|swe|backend|frontend|full[- ]?stack|machine learning|ml|artificial intelligence|ai|data|computer vision|nlp|embedded|robotics)\b",
    )
    .expect("valid regex")
});

#[derive(Debug, serde::Deserialize)]
struct RawSimplifyJob {
    id: String,
    company_name: String,
    title: String,
    #[serde(default)]
    category: Option<String>,
    active: bool,
    url: String,
    locations: Vec<String>,
    date_posted: i64,
    date_updated: i64,
    #[serde(default)]
    degrees: Option<Vec<String>>,
    #[serde(default)]
    is_visible: Option<bool>,
}

pub fn listings_url(season_year: u16) -> String {
    format!(
        "https://raw.githubusercontent.com/SimplifyJobs/Summer{season_year}-Internships/dev/.github/scripts/listings.json"
    )
}

pub async fn download(
    client: &reqwest::Client,
    season_year: u16,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = listings_url(season_year);
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await?
        .error_for_status()?;
    let list = resp.json::<Vec<serde_json::Value>>().await?;
    Ok(list)
}

pub fn filter_and_normalize(raw: &[serde_json::Value]) -> Vec<NewJob> {
    let mut jobs = Vec::new();
    for val in raw {
        let Ok(job) = serde_json::from_value::<RawSimplifyJob>(val.clone()) else {
            continue;
        };

        if !job.active {
            continue;
        }

        if job.is_visible == Some(false) {
            continue;
        }

        if !is_relevant_title(&job.title) {
            continue;
        }

        let degree_ok = match &job.degrees {
            None => true,
            Some(degs) => {
                degs.is_empty()
                    || degs
                        .iter()
                        .any(|d| d.trim().replace('\u{2019}', "'").eq_ignore_ascii_case("bachelor's"))
            }
        };
        if !degree_ok {
            continue;
        }

        if !job.locations.iter().any(|loc| is_us_location(loc)) {
            continue;
        }

        let provider = provider_of(&job.url);

        jobs.push(NewJob {
            simplify_id: job.id,
            company: job.company_name,
            title: job.title,
            category: job.category.unwrap_or_default(),
            url: job.url,
            provider,
            locations: job.locations,
            date_posted: job.date_posted,
            date_updated: job.date_updated,
        });
    }
    jobs
}

pub fn is_us_location(loc: &str) -> bool {
    let trimmed = loc.trim();
    let lower = trimmed.to_ascii_lowercase();

    // 1. lowercase contains united states or usa
    if lower.contains("united states") || lower.contains("usa") {
        return true;
    }

    // 2. the whole string is one of NYC, SF, South SF, LA, DC, Remote in USA
    if matches!(trimmed, "NYC" | "SF" | "South SF" | "LA" | "DC" | "Remote in USA") {
        return true;
    }

    // 3. the whole trimmed string (case-insensitive) is a full US state name
    if US_STATE_NAMES.contains(&lower.as_str()) {
        return true;
    }

    // 4. the last comma part (trimmed, uppercased) is a 2-letter US state code or DC
    if let Some((_, last)) = loc.rsplit_once(',') {
        let code = last.trim().to_ascii_uppercase();
        if US_STATE_CODES.contains(&code.as_str()) {
            return true;
        }
    }

    false
}

pub fn is_relevant_title(title: &str) -> bool {
    RELEVANT_TITLE_RE.is_match(title)
}

pub fn provider_of(url: &str) -> Provider {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return Provider::Other;
    };
    let Some(host) = parsed.host_str() else {
        return Provider::Other;
    };
    let host = host.to_ascii_lowercase();
    if host == "job-boards.greenhouse.io" || host == "boards.greenhouse.io" {
        Provider::Greenhouse
    } else if host == "jobs.ashbyhq.com" {
        Provider::Ashby
    } else if host == "jobs.lever.co" || host == "jobs.eu.lever.co" {
        Provider::Lever
    } else if host.ends_with(".myworkdayjobs.com") {
        Provider::Workday
    } else {
        Provider::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_locations() {
        for l in [
            "New York, NY",
            "NYC",
            "SF",
            "South SF",
            "California",
            "Remote in USA",
            "United States",
            "Washington, DC",
        ] {
            assert!(is_us_location(l), "{l}");
        }
        for l in [
            "London, UK",
            "Toronto, ON, Canada",
            "Montreal, QC, Canada",
            "Paris, France",
            "Remote in Canada",
        ] {
            assert!(!is_us_location(l), "{l}");
        }
    }

    #[test]
    fn titles() {
        assert!(is_relevant_title("Machine Learning Engineer Intern"));
        assert!(is_relevant_title("Intern Data Engineer"));
        assert!(!is_relevant_title("Product Marketing Intern"));
        assert!(!is_relevant_title("Systems Engineering Intern"));
        assert!(!is_relevant_title("Email Campaign Intern")); // "ai" must be a whole word, not inside "campaign"/"email"
    }

    #[test]
    fn providers() {
        assert_eq!(
            provider_of("https://boards.greenhouse.io/embed/job_app?token=1"),
            Provider::Greenhouse
        );
        assert_eq!(
            provider_of("https://job-boards.eu.greenhouse.io/a/jobs/1"),
            Provider::Other
        );
        assert_eq!(
            provider_of("https://jobs.eu.lever.co/a/b/apply"),
            Provider::Lever
        );
        assert_eq!(
            provider_of("https://pg.wd5.myworkdayjobs.com/1000/job/x/y"),
            Provider::Workday
        );
        assert_eq!(provider_of("not a url"), Provider::Other);
    }

    #[test]
    fn sample_filter() {
        let raw: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/simplify_sample.json")).unwrap();
        let mut ids: Vec<String> = filter_and_normalize(&raw).into_iter().map(|j| j.title).collect();
        ids.sort();
        // expected kept (verified against the rules): Neuralink ML (empty degrees, Fremont CA), Axiomatic SWE (Boston MA),
        // Circleback SWE (location "SF"), TikTok SWE (San Jose CA), Oracle "Intern Data Engineer" (empty degrees, United States)
        assert_eq!(
            ids,
            vec![
                "Intern Data Engineer",
                "Machine Learning Engineer Intern",
                "Software Engineer Intern",
                "Software Engineer Project Intern - Model Infrastructure",
                "Software Engineering Internship \u{2013} AI/Agentic Systems"
            ]
        );
    }

    #[test]
    fn malformed_entries_skipped() {
        let raw: Vec<serde_json::Value> = serde_json::from_str(r#"[{"id":1},{"oops":true}]"#).unwrap();
        assert!(filter_and_normalize(&raw).is_empty());
    }

    #[test]
    fn url_for_season() {
        assert!(listings_url(2028).contains("Summer2028-Internships"));
    }
}
