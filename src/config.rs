#![allow(dead_code)]

use std::path::Path;
use serde::Deserialize;

fn default_season_year() -> u16 {
    2027
}

fn default_api_key_env() -> String {
    "LLM_API_KEY".to_string()
}

fn default_requests_per_minute() -> u32 {
    10
}

fn default_concurrency() -> usize {
    8
}

fn default_per_host() -> usize {
    2
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_season_year")]
    pub season_year: u16,
    pub profile: Profile,
    pub llm: Option<LlmConfig>,
    #[serde(default)]
    pub scan: ScanConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Profile {
    pub class_year: u8,
    pub expected_graduation: Option<u16>,
    pub gpa: Option<f32>,
    pub needs_sponsorship: bool,
    pub has_prior_internship: bool,
}

impl Profile {
    pub fn grad_year(&self, season_year: u16) -> u16 {
        self.expected_graduation
            .unwrap_or_else(|| season_year + (4 - self.class_year as u16))
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_requests_per_minute")]
    pub requests_per_minute: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ScanConfig {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_per_host")]
    pub per_host: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            per_host: default_per_host(),
        }
    }
}

pub fn load(path: &Path) -> anyhow::Result<Config> {
    if !path.exists() {
        anyhow::bail!("scraper.toml not found; copy scraper.example.toml to scraper.toml and edit [profile]");
    }
    let content = std::fs::read_to_string(path)?;
    parse(&content)
}

pub fn parse(toml_text: &str) -> anyhow::Result<Config> {
    let config: Config = toml::from_str(toml_text)?;
    if !(1..=4).contains(&config.profile.class_year) {
        anyhow::bail!(
            "class_year must be between 1 and 4, got {}",
            config.profile.class_year
        );
    }
    if let Some(gpa) = config.profile.gpa {
        if !(0.0..=4.0).contains(&gpa) {
            anyhow::bail!("gpa must be between 0.0 and 4.0, got {}", gpa);
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_profile() {
        let c = parse(include_str!("../scraper.example.toml")).unwrap();
        assert_eq!(c.season_year, 2027);
        assert_eq!(c.profile.class_year, 1);
        assert_eq!(c.profile.gpa, None);
        assert!(c.profile.needs_sponsorship && !c.profile.has_prior_internship);
        assert_eq!(c.profile.grad_year(2027), 2030);
        assert!(c.llm.is_none());
        assert_eq!((c.scan.concurrency, c.scan.per_host), (8, 2));
    }

    #[test]
    fn explicit_graduation_wins() {
        let toml_str = r#"
            [profile]
            class_year = 2
            expected_graduation = 2031
            needs_sponsorship = false
            has_prior_internship = true
        "#;
        let c = parse(toml_str).unwrap();
        assert_eq!(c.profile.grad_year(2027), 2031);
    }

    #[test]
    fn class_year_out_of_range_errors() {
        let toml_str = r#"
            [profile]
            class_year = 5
            needs_sponsorship = false
            has_prior_internship = false
        "#;
        let err = parse(toml_str).unwrap_err().to_string();
        assert!(err.contains("class_year"), "expected error containing 'class_year', got: {err}");
    }

    #[test]
    fn gpa_out_of_range_errors() {
        let toml_str = r#"
            [profile]
            class_year = 1
            gpa = 4.5
            needs_sponsorship = false
            has_prior_internship = false
        "#;
        let err = parse(toml_str).unwrap_err().to_string();
        assert!(err.contains("gpa"), "expected error containing 'gpa', got: {err}");
    }

    #[test]
    fn llm_defaults() {
        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = false
            has_prior_internship = false

            [llm]
            base_url = "https://example.com/v1"
            model = "test-model"
        "#;
        let c = parse(toml_str).unwrap();
        let llm = c.llm.expect("expected llm to be Some");
        assert_eq!(llm.api_key_env, "LLM_API_KEY");
        assert_eq!(llm.requests_per_minute, 10);
    }

    #[test]
    fn load_missing_file_error() {
        let err = load(Path::new("non_existent_file.toml")).unwrap_err().to_string();
        assert_eq!(
            err,
            "scraper.toml not found; copy scraper.example.toml to scraper.toml and edit [profile]"
        );
    }
}
