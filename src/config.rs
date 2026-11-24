#![allow(dead_code)]

use std::path::{Path, PathBuf};
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

fn default_recheck_days() -> u32 {
    7
}

fn default_recheck_max() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ScanConfig {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_per_host")]
    pub per_host: usize,
    #[serde(default = "default_recheck_days")]
    pub recheck_days: u32,
    #[serde(default = "default_recheck_max")]
    pub recheck_max: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            per_host: default_per_host(),
            recheck_days: default_recheck_days(),
            recheck_max: default_recheck_max(),
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

pub const ENV_STARTER_TEMPLATE: &str = "# Uncomment and paste your key to enable the LLM pass (see [llm] in scraper.toml):\n# LLM_API_KEY=\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    pub config_path: PathBuf,
    pub config_created: bool,
    pub env_path: PathBuf,
    pub env_created: bool,
}

pub fn home_config_dir_from(
    appdata: Option<&str>,
    xdg: Option<&str>,
    home: Option<&str>,
    windows: bool,
) -> Option<PathBuf> {
    if windows {
        appdata
            .filter(|s| !s.trim().is_empty())
            .map(|s| PathBuf::from(s).join("scraper"))
    } else {
        if let Some(xdg_val) = xdg.filter(|s| !s.trim().is_empty()) {
            return Some(PathBuf::from(xdg_val).join("scraper"));
        }
        home.filter(|s| !s.trim().is_empty())
            .map(|s| PathBuf::from(s).join(".config").join("scraper"))
    }
}

pub fn home_config_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    home_config_dir_from(
        appdata.as_deref(),
        xdg.as_deref(),
        home.as_deref(),
        cfg!(windows),
    )
}

pub fn resolve_config_path(
    explicit: Option<&Path>,
    cwd: &Path,
    home_dir: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.exists() {
            anyhow::bail!("scraper.toml not found; copy scraper.example.toml to scraper.toml and edit [profile]");
        }
        return Ok(p.to_path_buf());
    }

    let cwd_path = cwd.join("scraper.toml");
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    if let Some(home) = home_dir {
        let home_path = home.join("scraper.toml");
        if home_path.exists() {
            return Ok(home_path);
        }
        anyhow::bail!(
            "no scraper.toml found in {} or {}; run `scraper init` to create one",
            cwd.display(),
            home_path.display()
        );
    }

    anyhow::bail!(
        "no scraper.toml found in {}; run `scraper init` to create one",
        cwd.display()
    );
}

pub fn resolve_db_path(explicit: Option<&Path>, config_path: &Path) -> PathBuf {
    if let Some(db) = explicit {
        db.to_path_buf()
    } else {
        let parent = config_path.parent().unwrap_or(Path::new(""));
        if parent.as_os_str().is_empty() {
            PathBuf::from("scraper.db")
        } else {
            parent.join("scraper.db")
        }
    }
}

pub fn init(home_dir: &Path) -> anyhow::Result<InitReport> {
    std::fs::create_dir_all(home_dir)?;
    let config_path = home_dir.join("scraper.toml");
    let config_created = if config_path.exists() {
        false
    } else {
        std::fs::write(&config_path, include_str!("../scraper.example.toml"))?;
        true
    };

    let env_path = home_dir.join(".env");
    let env_created = if env_path.exists() {
        false
    } else {
        std::fs::write(&env_path, ENV_STARTER_TEMPLATE)?;
        true
    };

    Ok(InitReport {
        config_path,
        config_created,
        env_path,
        env_created,
    })
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

    #[test]
    fn config_recheck_defaults() {
        let c = parse(include_str!("../scraper.example.toml")).unwrap();
        assert_eq!(c.scan.recheck_days, 7);
        assert_eq!(c.scan.recheck_max, 200);

        let toml_str = r#"
            [profile]
            class_year = 1
            needs_sponsorship = false
            has_prior_internship = false

            [scan]
            recheck_days = 0
        "#;
        let c2 = parse(toml_str).unwrap();
        assert_eq!(c2.scan.recheck_days, 0);
        assert_eq!(c2.scan.recheck_max, 200);

        let def = ScanConfig::default();
        assert_eq!(def.recheck_days, 7);
        assert_eq!(def.recheck_max, 200);
    }

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "scraper_test_{}_{}_{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_resolve_config_explicit_wins_even_if_cwd_has_one() {
        let cwd_guard = TempDirGuard::new("explicit_cwd");
        let cwd = cwd_guard.path();
        std::fs::write(cwd.join("scraper.toml"), "cwd = true").unwrap();

        let explicit_guard = TempDirGuard::new("explicit_dir");
        let explicit = explicit_guard.path().join("custom.toml");
        std::fs::write(&explicit, "explicit = true").unwrap();

        let res = resolve_config_path(Some(&explicit), cwd, None).unwrap();
        assert_eq!(res, explicit);

        let missing = explicit_guard.path().join("missing.toml");
        let err = resolve_config_path(Some(&missing), cwd, None).unwrap_err().to_string();
        assert_eq!(
            err,
            "scraper.toml not found; copy scraper.example.toml to scraper.toml and edit [profile]"
        );
    }

    #[test]
    fn test_resolve_config_cwd_wins_over_home() {
        let cwd_guard = TempDirGuard::new("cwd_wins_cwd");
        let cwd = cwd_guard.path();
        std::fs::write(cwd.join("scraper.toml"), "cwd = true").unwrap();

        let home_guard = TempDirGuard::new("cwd_wins_home");
        let home = home_guard.path();
        std::fs::write(home.join("scraper.toml"), "home = true").unwrap();

        let res = resolve_config_path(None, cwd, Some(home)).unwrap();
        assert_eq!(res, cwd.join("scraper.toml"));
    }

    #[test]
    fn test_resolve_config_home_used_when_cwd_has_none() {
        let cwd_guard = TempDirGuard::new("home_used_cwd");
        let cwd = cwd_guard.path();

        let home_guard = TempDirGuard::new("home_used_home");
        let home = home_guard.path();
        std::fs::write(home.join("scraper.toml"), "home = true").unwrap();

        let res = resolve_config_path(None, cwd, Some(home)).unwrap();
        assert_eq!(res, home.join("scraper.toml"));
    }

    #[test]
    fn test_resolve_config_neither_errors() {
        let cwd_guard = TempDirGuard::new("neither_cwd");
        let cwd = cwd_guard.path();

        let home_guard = TempDirGuard::new("neither_home");
        let home = home_guard.path();

        let err = resolve_config_path(None, cwd, Some(home)).unwrap_err().to_string();
        assert!(err.contains("scraper init"), "expected 'scraper init' in error, got: {err}");
        assert!(err.contains(&cwd.display().to_string()), "expected cwd in error, got: {err}");
        assert!(
            err.contains(&home.join("scraper.toml").display().to_string())
                || err.contains(&home.display().to_string()),
            "expected home path in error, got: {err}"
        );

        // When home_dir is None
        let err_no_home = resolve_config_path(None, cwd, None).unwrap_err().to_string();
        assert!(err_no_home.contains("scraper init"), "expected 'scraper init' in error, got: {err_no_home}");
        assert!(err_no_home.contains(&cwd.display().to_string()), "expected cwd in error, got: {err_no_home}");
    }

    #[test]
    fn test_resolve_db_path() {
        let config_path = Path::new("/some/dir/scraper.toml");
        let db = resolve_db_path(None, config_path);
        assert_eq!(db, Path::new("/some/dir/scraper.db"));

        let cwd_config = Path::new("scraper.toml");
        let db_cwd = resolve_db_path(None, cwd_config);
        assert_eq!(db_cwd, Path::new("scraper.db"));

        let explicit = Path::new("/custom/location/data.sqlite");
        let db_explicit = resolve_db_path(Some(explicit), config_path);
        assert_eq!(db_explicit, explicit);
    }

    #[test]
    fn test_init_creates_files_and_idempotent() {
        let home_guard = TempDirGuard::new("init_test");
        let home = home_guard.path();

        // 1. Initial init
        let report = init(home).unwrap();
        assert_eq!(report.config_path, home.join("scraper.toml"));
        assert!(report.config_created);
        assert_eq!(report.env_path, home.join(".env"));
        assert!(report.env_created);

        // Check content
        let cfg_content = std::fs::read_to_string(&report.config_path).unwrap();
        assert_eq!(cfg_content, include_str!("../scraper.example.toml"));

        let env_content = std::fs::read_to_string(&report.env_path).unwrap();
        let expected_env = "# Uncomment and paste your key to enable the LLM pass (see [llm] in scraper.toml):\n# LLM_API_KEY=\n";
        assert_eq!(env_content, expected_env);

        // Edit both files
        std::fs::write(&report.config_path, "custom_config = true").unwrap();
        std::fs::write(&report.env_path, "LLM_API_KEY=real_secret_key\n").unwrap();

        // 2. Second init: leaves both edited files unchanged
        let report2 = init(home).unwrap();
        assert_eq!(report2.config_path, home.join("scraper.toml"));
        assert!(!report2.config_created);
        assert_eq!(report2.env_path, home.join(".env"));
        assert!(!report2.env_created);

        assert_eq!(std::fs::read_to_string(&report.config_path).unwrap(), "custom_config = true");
        assert_eq!(std::fs::read_to_string(&report.env_path).unwrap(), "LLM_API_KEY=real_secret_key\n");
    }

    #[test]
    fn test_init_existing_env_with_real_key_never_overwritten() {
        let home_guard = TempDirGuard::new("init_existing_env");
        let home = home_guard.path();
        std::fs::create_dir_all(home).unwrap();
        let env_file = home.join(".env");
        std::fs::write(&env_file, "LLM_API_KEY=sk-already-exists-12345\n").unwrap();

        let report = init(home).unwrap();
        assert!(report.config_created);
        assert!(!report.env_created);
        assert_eq!(
            std::fs::read_to_string(&env_file).unwrap(),
            "LLM_API_KEY=sk-already-exists-12345\n"
        );
    }

    #[test]
    fn test_home_config_dir_from_matrix() {
        // Windows
        let win_path = home_config_dir_from(Some("C:\\Users\\alice\\AppData\\Roaming"), None, None, true);
        assert_eq!(win_path, Some(PathBuf::from("C:\\Users\\alice\\AppData\\Roaming\\scraper")));

        assert_eq!(home_config_dir_from(None, None, None, true), None);
        assert_eq!(home_config_dir_from(Some(""), None, None, true), None);
        assert_eq!(home_config_dir_from(Some("  "), None, None, true), None);

        // Non-Windows with XDG
        let xdg_path = home_config_dir_from(None, Some("/custom/xdg"), Some("/home/alice"), false);
        assert_eq!(xdg_path, Some(PathBuf::from("/custom/xdg/scraper")));

        // Non-Windows with empty XDG (ignored) -> falls back to HOME
        let xdg_empty = home_config_dir_from(None, Some(""), Some("/home/alice"), false);
        assert_eq!(xdg_empty, Some(PathBuf::from("/home/alice/.config/scraper")));

        let xdg_spaces = home_config_dir_from(None, Some("   "), Some("/home/alice"), false);
        assert_eq!(xdg_spaces, Some(PathBuf::from("/home/alice/.config/scraper")));

        // Non-Windows without XDG -> HOME
        let home_only = home_config_dir_from(None, None, Some("/home/alice"), false);
        assert_eq!(home_only, Some(PathBuf::from("/home/alice/.config/scraper")));

        // Non-Windows without XDG and without HOME -> None
        assert_eq!(home_config_dir_from(None, None, None, false), None);
        assert_eq!(home_config_dir_from(None, Some(""), Some(""), false), None);
        assert_eq!(home_config_dir_from(None, None, Some(""), false), None);
    }
}
