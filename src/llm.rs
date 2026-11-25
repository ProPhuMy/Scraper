#![allow(dead_code)]

use std::collections::BTreeSet;
use std::time::Duration;
use crate::config::LlmConfig;
use crate::model::{
    Citizenship, ClassYears, Evidence, Experience, Fact, Facts, Gpa, PortalPage, Source,
    Sponsorship,
};
use crate::portals::USER_AGENT;
use crate::text::truncate_chars;

#[derive(Debug, Clone)]
pub struct PacingState {
    pub requests_per_minute: u32,
    pub last_request_at: Option<std::time::Instant>,
    pub remaining_tokens: Option<u64>,
    pub reset_tokens: Option<Duration>,
    pub remaining_requests: Option<u64>,
    pub reset_requests: Option<Duration>,
    pub last_total_tokens: u64,
}

impl Default for PacingState {
    fn default() -> Self {
        Self {
            requests_per_minute: 10,
            last_request_at: None,
            remaining_tokens: None,
            reset_tokens: None,
            remaining_requests: None,
            reset_requests: None,
            last_total_tokens: 2000,
        }
    }
}

pub fn build_chat_body(cfg: &LlmConfig, sys: &str, user: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "temperature": 0,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": user}
        ]
    });
    if let Some(ref effort) = cfg.reasoning_effort {
        body["reasoning_effort"] = serde_json::Value::String(effort.clone());
    }
    body
}

static DURATION_FULL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^(\d+(?:\.\d+)?(ms|h|m|s))+$").expect("valid regex")
});
static DURATION_PART_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(\d+(?:\.\d+)?)(ms|h|m|s)").expect("valid regex")
});

pub fn parse_duration(s: &str) -> Option<Duration> {
    let trimmed = s.trim();
    if trimmed.is_empty() || !DURATION_FULL_RE.is_match(trimmed) {
        return None;
    }

    let mut total_nanos: u128 = 0;
    let mut last_rank = 5;

    for caps in DURATION_PART_RE.captures_iter(trimmed) {
        let val: f64 = caps[1].parse().ok()?;
        let unit = &caps[2];
        let (rank, multiplier_nanos) = match unit {
            "h" => (4, 3600.0 * 1_000_000_000.0),
            "m" => (3, 60.0 * 1_000_000_000.0),
            "s" => (2, 1_000_000_000.0),
            "ms" => (1, 1_000_000.0),
            _ => return None,
        };
        if rank >= last_rank {
            return None;
        }
        last_rank = rank;
        let nanos = (val * multiplier_nanos).round() as u128;
        total_nanos = total_nanos.checked_add(nanos)?;
    }

    let secs = (total_nanos / 1_000_000_000) as u64;
    let subsec_nanos = (total_nanos % 1_000_000_000) as u32;
    Some(Duration::new(secs, subsec_nanos))
}

pub fn pacing_wait(state: &PacingState, now: std::time::Instant) -> Duration {
    let mut wait = Duration::ZERO;

    // Minimum spacing since previous request
    let rpm = state.requests_per_minute.max(1);
    let min_spacing = Duration::from_millis(60_000 / rpm as u64);
    if let Some(prev) = state.last_request_at {
        let elapsed = now.saturating_duration_since(prev);
        let rpm_wait = min_spacing.saturating_sub(elapsed);
        if rpm_wait > wait {
            wait = rpm_wait;
        }
    }

    // Check remaining tokens
    let last_total = if state.last_total_tokens == 0 {
        2000
    } else {
        state.last_total_tokens
    };
    if let Some(remaining_tokens) = state.remaining_tokens {
        if remaining_tokens < last_total {
            if let Some(reset) = state.reset_tokens {
                let token_wait = reset + Duration::from_millis(200);
                if token_wait > wait {
                    wait = token_wait;
                }
            }
        }
    }

    // Check remaining requests
    if let Some(remaining_requests) = state.remaining_requests {
        if remaining_requests == 0 {
            if let Some(reset) = state.reset_requests {
                let req_wait = reset + Duration::from_millis(200);
                if req_wait > wait {
                    wait = req_wait;
                }
            }
        }
    }

    wait
}

pub fn per_minute_retry_wait(retry_after: Option<&str>, body: &str) -> Duration {
    // 1. check retry-after header (seconds)
    if let Some(val) = retry_after {
        let trimmed = val.trim();
        if let Ok(secs) = trimmed.parse::<u64>() {
            return Duration::from_secs(secs).min(Duration::from_secs(120));
        } else if let Some(dur) = parse_duration(trimmed) {
            return dur.min(Duration::from_secs(120));
        }
    }

    // 2. parse "Please try again in <duration>" from body
    if let Some(idx) = body.find("Please try again in ") {
        let rest = &body[idx + "Please try again in ".len()..];
        if let Some(caps) = DURATION_PART_RE.find(rest) {
            let mut end = caps.end();
            while let Some(next_caps) = DURATION_PART_RE.find(&rest[end..]) {
                if next_caps.start() == 0 {
                    end += next_caps.end();
                } else {
                    break;
                }
            }
            if let Some(dur) = parse_duration(&rest[..end]) {
                return dur.min(Duration::from_secs(120));
            }
        }
    }

    // 3. fallback 60s
    Duration::from_secs(60)
}

static EXP_BLOCKING_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(internship|internships|work experience|industry|professional|employment|years?)\b")
        .expect("valid regex")
});

static OPT_BLOCKING_WORD_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bopt\b").expect("valid regex")
});

static CLASS_YEARS_BLOCKING_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(graduat\w*|class of|freshm\w*|sophomores?|juniors?|seniors?|first-year|second-year|third-year|fourth-year|20\d\d)\b")
        .expect("valid regex")
});

static CITIZENSHIP_BLOCKING_REQ_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\b(must|required|requires|require|only|limited to|clearance|itar)\b")
        .expect("valid regex")
});

static CLASS_YEARS_PREFERRED_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\b(preferred|preference|preferably|ideally|a plus|desired|nice to have)\b")
        .expect("valid regex")
});

pub fn blocking_evidence_ok(fact: &str, value: &str, evidence: &str) -> bool {
    let norm = crate::text::normalize(evidence);
    match fact {
        "experience" => {
            if value == "required" {
                EXP_BLOCKING_RE.is_match(&norm)
            } else {
                true
            }
        }
        "citizenship" => {
            if value == "required" {
                let has_keyword = norm.contains("citizen")
                    || norm.contains("clearance")
                    || norm.contains("us person")
                    || norm.contains("u.s. person")
                    || norm.contains("permanent resident")
                    || norm.contains("green card")
                    || norm.contains("itar")
                    || norm.contains("export");
                if !has_keyword {
                    return false;
                }
                let cleaned = norm.replace("information is required", " ");
                CITIZENSHIP_BLOCKING_REQ_RE.is_match(&cleaned)
            } else {
                true
            }
        }
        "sponsorship" => {
            if value == "not_offered" {
                // "visa" / "work authorization" alone are NOT enough: "if an international student,
                // must already have a work or study visa" welcomes F-1 students (real false "no").
                norm.contains("sponsor")
                    || norm.contains("cpt")
                    || OPT_BLOCKING_WORD_RE.is_match(&norm)
                    || norm.contains("citizen")
                    || norm.contains("permanent resident")
                    || norm.contains("green card")
                    || norm.contains("for any employer")
            } else {
                true
            }
        }
        "gpa" => {
            if value == "min" || value.parse::<f64>().is_ok() {
                norm.contains("gpa") || norm.contains("grade point")
            } else {
                true
            }
        }
        "class_years" => {
            if value != "any" && value != "not_mentioned" && value != "unknown" {
                // "juniors or seniors preferred" is not a requirement (real false "no").
                CLASS_YEARS_BLOCKING_RE.is_match(&norm) && !CLASS_YEARS_PREFERRED_RE.is_match(&norm)
            } else {
                true
            }
        }
        _ => true,
    }
}

pub struct LlmClient {
    client: reqwest::Client,
    cfg: LlmConfig,
    api_key: String,
    pacing: std::sync::Mutex<PacingState>,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("cfg", &self.cfg)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimit {
    PerMinute,
    Daily(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmOutcome {
    Facts(Facts),
    QuotaExhausted(String),
}

impl LlmClient {
    pub fn from_config(cfg: &LlmConfig) -> anyhow::Result<LlmClient> {
        dotenvy::dotenv().ok();
        let key = std::env::var(&cfg.api_key_env)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if key.is_empty() {
            anyhow::bail!("set {} in the environment or .env", cfg.api_key_env);
        }
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()?;
        let pacing = PacingState {
            requests_per_minute: cfg.requests_per_minute,
            last_request_at: None,
            remaining_tokens: None,
            reset_tokens: None,
            remaining_requests: None,
            reset_requests: None,
            last_total_tokens: 2000,
        };
        Ok(LlmClient {
            client,
            cfg: cfg.clone(),
            api_key: key,
            pacing: std::sync::Mutex::new(pacing),
        })
    }

    pub async fn settle(
        &self,
        title: &str,
        page: &PortalPage,
        facts: &Facts,
        season_year: u16,
    ) -> anyhow::Result<LlmOutcome> {
        let (sys, user) = build_messages(title, page, facts, season_year);
        let url = format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let body = build_chat_body(&self.cfg, &sys, &user);

        let mut attempt = 0;
        loop {
            attempt += 1;
            let wait = {
                let state = self.pacing.lock().unwrap_or_else(|e| e.into_inner());
                pacing_wait(&state, std::time::Instant::now())
            };
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }

            {
                let mut state = self.pacing.lock().unwrap_or_else(|e| e.into_inner());
                state.last_request_at = Some(std::time::Instant::now());
                state.remaining_tokens = None;
                state.reset_tokens = None;
                state.remaining_requests = None;
                state.reset_requests = None;
            }

            let req = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let text = resp.text().await.unwrap_or_default();

                    // Read rate limit headers after every response (success or 429)
                    {
                        let mut state = self.pacing.lock().unwrap_or_else(|e| e.into_inner());
                        state.remaining_tokens = headers
                            .get("x-ratelimit-remaining-tokens")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok());
                        state.reset_tokens = headers
                            .get("x-ratelimit-reset-tokens")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok().map(Duration::from_secs).or_else(|| parse_duration(s)));
                        state.remaining_requests = headers
                            .get("x-ratelimit-remaining-requests")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok());
                        state.reset_requests = headers
                            .get("x-ratelimit-reset-requests")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok().map(Duration::from_secs).or_else(|| parse_duration(s)));
                    }

                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        anyhow::bail!("LLM rejected the API key");
                    }

                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        match classify_429(&text) {
                            RateLimit::Daily(d) => return Ok(LlmOutcome::QuotaExhausted(d)),
                            RateLimit::PerMinute => {
                                if attempt <= 3 {
                                    let retry_after = headers
                                        .get(reqwest::header::RETRY_AFTER)
                                        .and_then(|v| v.to_str().ok());
                                    let retry_wait = per_minute_retry_wait(retry_after, &text);
                                    tokio::time::sleep(retry_wait).await;
                                    continue;
                                } else {
                                    anyhow::bail!("HTTP 429 after retry: {text}");
                                }
                            }
                        }
                    }

                    if status.is_server_error() {
                        if attempt == 1 {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        } else {
                            anyhow::bail!("HTTP {status} after retry: {text}");
                        }
                    }

                    if !status.is_success() {
                        anyhow::bail!("HTTP {status}: {text}");
                    }

                    let val: serde_json::Value = serde_json::from_str(&text)?;

                    // Remember usage.total_tokens of last successful response
                    if let Some(total_tokens) = val
                        .get("usage")
                        .and_then(|u| u.get("total_tokens"))
                        .and_then(|t| t.as_u64())
                    {
                        let mut state = self.pacing.lock().unwrap_or_else(|e| e.into_inner());
                        state.last_total_tokens = total_tokens;
                    }

                    let content = val
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("message"))
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "malformed LLM response: missing choices[0].message.content"
                            )
                        })?;

                    let settled = parse_answer(content, facts, title, page)?;
                    return Ok(LlmOutcome::Facts(settled));
                }
                Err(e) => {
                    if attempt == 1 {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    } else {
                        anyhow::bail!("LLM request error: {e}");
                    }
                }
            }
        }
    }
}

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You classify internship eligibility facts from a job posting. Answer ONLY with a JSON object.
Use ONLY the facts listed in the user message. For each, return {\"value\": ..., \"evidence\": \"<exact quote from the posting, max 200 chars>\"}.
If the posting does not settle a fact, return \"unknown\" for it. Never guess.
Allowed values:
- class_years: an array of accepted graduation years (integers) ONLY if the posting names graduation years, \"class of\" years, or class standings (freshman/sophomore/junior/senior, first-year, rising ...). The internship is in summer {S}. A current freshman graduates {S+3}, sophomore {S+2}, junior {S+1}. \"Current students\", \"enrolled in a degree program\" or \"undergraduates\" alone means \"any\". Years or standings that are only preferred (\"juniors or seniors preferred\") are \"unknown\". Else \"unknown\".
- gpa: a number (the required minimum), \"asked\", \"not_mentioned\", or \"unknown\".
- sponsorship: \"not_offered\" ONLY if the posting refuses CPT/OPT, is limited to US citizens/permanent residents, or requires no sponsorship now or in the future. Plain \"we do not sponsor visas\" or a question asking about sponsorship is \"asked\". \"offered\" if sponsorship or CPT/OPT is explicitly welcome. Else \"not_mentioned\" or \"unknown\".
- citizenship: \"required\" ONLY if the posting states US citizenship, US person status, or a security clearance is required; a form question that merely asks about citizenship, nationality or status (export-control or I-9 paperwork) is \"unknown\". Else \"not_mentioned\" or \"unknown\". Ignore equal-opportunity boilerplate and \"authorized to work in the US\" questions.
- experience: \"required\" if prior internship/work experience is required, \"preferred\" if it is a plus. Skills like \"experience with Python\" do not count. Else \"not_mentioned\" or \"unknown\".";

static OPT_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\bopt\b").expect("valid regex"));

pub fn focus_excerpt(description: &str, unknown: &[&str]) -> String {
    let sents = crate::text::sentences(description);
    if sents.is_empty() {
        return truncate_chars(description, 4_000);
    }

    let check_sponsorship = unknown.contains(&"sponsorship");
    let check_citizenship = unknown.contains(&"citizenship");
    let check_gpa = unknown.contains(&"gpa");
    let check_experience = unknown.contains(&"experience");
    let check_class_years = unknown.contains(&"class_years");

    let mut keep = vec![false; sents.len()];
    let mut any_matched = false;

    for (i, sent) in sents.iter().enumerate() {
        let norm = crate::text::normalize(sent);
        let mut matched = false;

        if check_sponsorship
            && (norm.contains("sponsor")
                || norm.contains("visa")
                || norm.contains("h-1b")
                || norm.contains("h1b")
                || norm.contains("cpt")
                || OPT_RE.is_match(&norm)
                || norm.contains("f-1")
                || norm.contains("immigration")
                || norm.contains("work authorization")
                || norm.contains("authorized to work"))
        {
            matched = true;
        }

        if !matched
            && check_citizenship
            && (norm.contains("citizen")
                || norm.contains("clearance")
                || norm.contains("us person")
                || norm.contains("itar")
                || norm.contains("export")
                || norm.contains("permanent resident")
                || norm.contains("green card"))
        {
            matched = true;
        }

        if !matched && check_gpa && (norm.contains("gpa") || norm.contains("grade point")) {
            matched = true;
        }

        if !matched
            && check_experience
            && (norm.contains("experience") || norm.contains("internship"))
        {
            matched = true;
        }

        if !matched
            && check_class_years
            && (norm.contains("freshm")
                || norm.contains("sophomore")
                || norm.contains("junior")
                || norm.contains("senior")
                || norm.contains("graduat")
                || norm.contains("class of")
                || norm.contains("enrolled")
                || norm.contains("undergraduate")
                || norm.contains("student")
                || norm.contains("degree"))
        {
            matched = true;
        }

        if matched {
            any_matched = true;
            if i > 0 {
                keep[i - 1] = true;
            }
            keep[i] = true;
            if i + 1 < sents.len() {
                keep[i + 1] = true;
            }
        }
    }

    if !any_matched {
        return truncate_chars(description, 4_000);
    }

    let kept_sents: Vec<&str> = sents
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if keep[i] { Some(s.as_str()) } else { None })
        .collect();

    truncate_chars(&kept_sents.join("\n"), 4_000)
}

/// Max options shown per form question. Real forms carry university / country / state
/// dropdowns with thousands of entries (Palantir: 3,302 universities), which pushed single
/// requests to 17-24K tokens and got HTTP 413 from Groq's 8K tokens-per-minute free tier.
const MAX_OPTIONS_PER_QUESTION: usize = 15;
/// Hard cap for the whole form-questions section.
const MAX_QUESTIONS_CHARS: usize = 3_000;

pub fn format_questions(questions: &[crate::model::FormQuestion]) -> String {
    let mut out = String::new();
    for q in questions {
        let line = if q.options.is_empty() {
            format!("- {}\n", q.label)
        } else if q.options.len() > MAX_OPTIONS_PER_QUESTION {
            format!(
                "- {} [options: {} | …(+{} more)]\n",
                q.label,
                q.options[..MAX_OPTIONS_PER_QUESTION].join(" | "),
                q.options.len() - MAX_OPTIONS_PER_QUESTION
            )
        } else {
            format!("- {} [options: {}]\n", q.label, q.options.join(" | "))
        };
        if out.chars().count() + line.chars().count() > MAX_QUESTIONS_CHARS {
            out.push_str("- …(more questions omitted)\n");
            break;
        }
        out.push_str(&line);
    }
    out
}

pub fn build_messages(
    title: &str,
    page: &PortalPage,
    facts: &Facts,
    season_year: u16,
) -> (String, String) {
    let s = season_year;
    let sys = SYSTEM_PROMPT_TEMPLATE
        .replace("{S}", &s.to_string())
        .replace("{S+3}", &(s + 3).to_string())
        .replace("{S+2}", &(s + 2).to_string())
        .replace("{S+1}", &(s + 1).to_string());

    let mut user = format!(
        "Facts to settle: {}\nTitle: {}\nForm questions:\n",
        facts.unknown_names().join(", "),
        title
    );
    user.push_str(&format_questions(&page.questions));
    let excerpt = focus_excerpt(&page.description, &facts.unknown_names());
    user.push_str(&format!(
        "Relevant excerpts from the description:\n{}",
        excerpt
    ));

    (sys, user)
}

fn strip_code_fences(content: &str) -> &str {
    let s = content.trim();
    if s.starts_with("```") {
        let body = if let Some(idx) = s.find('\n') {
            &s[idx + 1..]
        } else {
            let inner = s.strip_prefix("```").unwrap_or(s);
            inner.strip_prefix("json").or_else(|| inner.strip_prefix("JSON")).unwrap_or(inner)
        };
        let body = body.trim();
        if let Some(idx) = body.rfind("```") {
            body[..idx].trim()
        } else {
            body
        }
    } else {
        s
    }
}

fn get_evidence(fact_val: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    fact_val
        .get("evidence")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn evidence_in_posting(evidence: &str, title: &str, page: &PortalPage) -> bool {
    let mut raw = String::new();
    raw.push_str(title);
    raw.push('\n');
    raw.push_str(&page.description);
    raw.push('\n');
    for q in &page.questions {
        raw.push_str(&q.label);
        raw.push('\n');
        for opt in &q.options {
            raw.push_str(opt);
            raw.push('\n');
        }
    }
    let haystack = collapse_whitespace(&crate::text::normalize(&raw));

    let norm_ev = collapse_whitespace(&crate::text::normalize(evidence));
    let mut needle = norm_ev.as_str();

    // Strip surrounding quotes ("'“”‘’) and trailing punctuation (.,;:)
    loop {
        let prev = needle;
        needle = needle.trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’'));
        needle = needle.trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ':'));
        needle = needle.trim();
        if needle == prev {
            break;
        }
    }

    // the LLM may shorten quotes with "..." or "…": split the needle on those;
    // EVERY piece that is at least 8 chars long must appear in the haystack;
    // pieces shorter than 8 chars are ignored; but if NO piece is at least 8 chars long, return false.
    let pieces: Vec<&str> = needle
        .split("...")
        .flat_map(|p| p.split('…'))
        .map(|p| {
            p.trim()
                .trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’'))
                .trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ':'))
                .trim()
        })
        .collect();

    let mut has_long_piece = false;
    for piece in pieces {
        if piece.chars().count() < 8 {
            continue;
        }
        has_long_piece = true;
        if !haystack.contains(piece) {
            return false;
        }
    }

    has_long_piece
}

fn get_valid_evidence(
    fact_val: &serde_json::Map<String, serde_json::Value>,
    title: &str,
    page: &PortalPage,
) -> Option<String> {
    let ev = get_evidence(fact_val)?;
    if evidence_in_posting(&ev, title, page) {
        Some(ev)
    } else {
        None
    }
}

pub fn parse_answer(
    content: &str,
    current: &Facts,
    title: &str,
    page: &PortalPage,
) -> anyhow::Result<Facts> {
    let cleaned = strip_code_fences(content);
    let parsed: serde_json::Value = serde_json::from_str(cleaned)?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("expected JSON object"))?;

    let mut result = current.clone();

    // 1. class_years
    if matches!(current.class_years.value, ClassYears::Unknown) && obj.contains_key("class_years") {
        if let Some(val_entry) = obj.get("class_years").and_then(|v| v.as_object()) {
            let v = val_entry.get("value");
            if v == Some(&serde_json::Value::String("unknown".to_string())) {
                result.class_years = Fact {
                    value: ClassYears::Unknown,
                    evidence: None,
                };
            } else if v == Some(&serde_json::Value::String("any".to_string())) {
                if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                    result.class_years = Fact {
                        value: ClassYears::Any,
                        evidence: Some(Evidence {
                            source: Source::Llm,
                            snippet: truncate_chars(&ev, 200),
                        }),
                    };
                } else {
                    result.class_years = Fact {
                        value: ClassYears::Unknown,
                        evidence: None,
                    };
                }
            } else if let Some(arr) = v.and_then(|v| v.as_array()) {
                let years: Option<BTreeSet<u16>> = if arr.is_empty() {
                    None
                } else {
                    let mut set = BTreeSet::new();
                    let mut valid = true;
                    for item in arr {
                        if let Some(y) = item.as_u64() {
                            if y <= u16::MAX as u64 {
                                set.insert(y as u16);
                            } else {
                                valid = false;
                                break;
                            }
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    if valid {
                        Some(set)
                    } else {
                        None
                    }
                };

                if let Some(years) = years {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        if blocking_evidence_ok("class_years", "grad", &ev) {
                            result.class_years = Fact {
                                value: ClassYears::Grad(years),
                                evidence: Some(Evidence {
                                    source: Source::Llm,
                                    snippet: truncate_chars(&ev, 200),
                                }),
                            };
                        } else {
                            result.class_years = Fact {
                                value: ClassYears::Unknown,
                                evidence: None,
                            };
                        }
                    } else {
                        result.class_years = Fact {
                            value: ClassYears::Unknown,
                            evidence: None,
                        };
                    }
                } else {
                    result.class_years = Fact {
                        value: ClassYears::Unknown,
                        evidence: None,
                    };
                }
            } else {
                result.class_years = Fact {
                    value: ClassYears::Unknown,
                    evidence: None,
                };
            }
        } else {
            result.class_years = Fact {
                value: ClassYears::Unknown,
                evidence: None,
            };
        }
    }

    // 2. gpa
    if matches!(current.gpa.value, Gpa::Unknown) && obj.contains_key("gpa") {
        if let Some(val_entry) = obj.get("gpa").and_then(|v| v.as_object()) {
            let v = val_entry.get("value");
            if v == Some(&serde_json::Value::String("unknown".to_string())) {
                result.gpa = Fact {
                    value: Gpa::Unknown,
                    evidence: None,
                };
            } else if v == Some(&serde_json::Value::String("asked".to_string())) {
                if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                    result.gpa = Fact {
                        value: Gpa::Asked,
                        evidence: Some(Evidence {
                            source: Source::Llm,
                            snippet: truncate_chars(&ev, 200),
                        }),
                    };
                } else {
                    result.gpa = Fact {
                        value: Gpa::Unknown,
                        evidence: None,
                    };
                }
            } else if v == Some(&serde_json::Value::String("not_mentioned".to_string())) {
                if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                    result.gpa = Fact {
                        value: Gpa::NotMentioned,
                        evidence: Some(Evidence {
                            source: Source::Llm,
                            snippet: truncate_chars(&ev, 200),
                        }),
                    };
                } else {
                    result.gpa = Fact {
                        value: Gpa::Unknown,
                        evidence: None,
                    };
                }
            } else if let Some(num) = v.and_then(|n| n.as_f64()).map(|f| f as f32) {
                if (1.0..=4.0).contains(&num) {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        if blocking_evidence_ok("gpa", &num.to_string(), &ev) {
                            result.gpa = Fact {
                                value: Gpa::Min(num),
                                evidence: Some(Evidence {
                                    source: Source::Llm,
                                    snippet: truncate_chars(&ev, 200),
                                }),
                            };
                        } else {
                            result.gpa = Fact {
                                value: Gpa::Unknown,
                                evidence: None,
                            };
                        }
                    } else {
                        result.gpa = Fact {
                            value: Gpa::Unknown,
                            evidence: None,
                        };
                    }
                } else {
                    result.gpa = Fact {
                        value: Gpa::Unknown,
                        evidence: None,
                    };
                }
            } else {
                result.gpa = Fact {
                    value: Gpa::Unknown,
                    evidence: None,
                };
            }
        } else {
            result.gpa = Fact {
                value: Gpa::Unknown,
                evidence: None,
            };
        }
    }

    // 3. sponsorship
    if matches!(current.sponsorship.value, Sponsorship::Unknown) && obj.contains_key("sponsorship") {
        if let Some(val_entry) = obj.get("sponsorship").and_then(|v| v.as_object()) {
            match val_entry.get("value").and_then(|s| s.as_str()) {
                Some("unknown") => {
                    result.sponsorship = Fact {
                        value: Sponsorship::Unknown,
                        evidence: None,
                    };
                }
                Some("not_offered") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        if blocking_evidence_ok("sponsorship", "not_offered", &ev) {
                            result.sponsorship = Fact {
                                value: Sponsorship::NotOffered,
                                evidence: Some(Evidence {
                                    source: Source::Llm,
                                    snippet: truncate_chars(&ev, 200),
                                }),
                            };
                        } else {
                            result.sponsorship = Fact {
                                value: Sponsorship::Unknown,
                                evidence: None,
                            };
                        }
                    } else {
                        result.sponsorship = Fact {
                            value: Sponsorship::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("offered") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.sponsorship = Fact {
                            value: Sponsorship::Offered,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.sponsorship = Fact {
                            value: Sponsorship::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("asked") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.sponsorship = Fact {
                            value: Sponsorship::Asked,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.sponsorship = Fact {
                            value: Sponsorship::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("not_mentioned") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.sponsorship = Fact {
                            value: Sponsorship::NotMentioned,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.sponsorship = Fact {
                            value: Sponsorship::Unknown,
                            evidence: None,
                        };
                    }
                }
                _ => {
                    result.sponsorship = Fact {
                        value: Sponsorship::Unknown,
                        evidence: None,
                    };
                }
            }
        } else {
            result.sponsorship = Fact {
                value: Sponsorship::Unknown,
                evidence: None,
            };
        }
    }

    // 4. citizenship
    if matches!(current.citizenship.value, Citizenship::Unknown) && obj.contains_key("citizenship") {
        if let Some(val_entry) = obj.get("citizenship").and_then(|v| v.as_object()) {
            match val_entry.get("value").and_then(|s| s.as_str()) {
                Some("unknown") => {
                    result.citizenship = Fact {
                        value: Citizenship::Unknown,
                        evidence: None,
                    };
                }
                Some("required") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        if blocking_evidence_ok("citizenship", "required", &ev) {
                            result.citizenship = Fact {
                                value: Citizenship::Required,
                                evidence: Some(Evidence {
                                    source: Source::Llm,
                                    snippet: truncate_chars(&ev, 200),
                                }),
                            };
                        } else {
                            result.citizenship = Fact {
                                value: Citizenship::Unknown,
                                evidence: None,
                            };
                        }
                    } else {
                        result.citizenship = Fact {
                            value: Citizenship::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("not_mentioned") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.citizenship = Fact {
                            value: Citizenship::NotMentioned,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.citizenship = Fact {
                            value: Citizenship::Unknown,
                            evidence: None,
                        };
                    }
                }
                _ => {
                    result.citizenship = Fact {
                        value: Citizenship::Unknown,
                        evidence: None,
                    };
                }
            }
        } else {
            result.citizenship = Fact {
                value: Citizenship::Unknown,
                evidence: None,
            };
        }
    }

    // 5. experience
    if matches!(current.experience.value, Experience::Unknown) && obj.contains_key("experience") {
        if let Some(val_entry) = obj.get("experience").and_then(|v| v.as_object()) {
            match val_entry.get("value").and_then(|s| s.as_str()) {
                Some("unknown") => {
                    result.experience = Fact {
                        value: Experience::Unknown,
                        evidence: None,
                    };
                }
                Some("required") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        if blocking_evidence_ok("experience", "required", &ev) {
                            result.experience = Fact {
                                value: Experience::Required,
                                evidence: Some(Evidence {
                                    source: Source::Llm,
                                    snippet: truncate_chars(&ev, 200),
                                }),
                            };
                        } else {
                            result.experience = Fact {
                                value: Experience::Unknown,
                                evidence: None,
                            };
                        }
                    } else {
                        result.experience = Fact {
                            value: Experience::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("preferred") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.experience = Fact {
                            value: Experience::Preferred,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.experience = Fact {
                            value: Experience::Unknown,
                            evidence: None,
                        };
                    }
                }
                Some("not_mentioned") => {
                    if let Some(ev) = get_valid_evidence(val_entry, title, page) {
                        result.experience = Fact {
                            value: Experience::NotMentioned,
                            evidence: Some(Evidence {
                                source: Source::Llm,
                                snippet: truncate_chars(&ev, 200),
                            }),
                        };
                    } else {
                        result.experience = Fact {
                            value: Experience::Unknown,
                            evidence: None,
                        };
                    }
                }
                _ => {
                    result.experience = Fact {
                        value: Experience::Unknown,
                        evidence: None,
                    };
                }
            }
        } else {
            result.experience = Fact {
                value: Experience::Unknown,
                evidence: None,
            };
        }
    }

    Ok(result)
}

pub fn format_class_years(years: &BTreeSet<u16>) -> String {
    let list: Vec<u16> = years.iter().copied().collect();
    if list.len() >= 2 && list.windows(2).all(|w| w[1] == w[0] + 1) {
        format!("{}-{}", list[0], list[list.len() - 1])
    } else {
        list.iter().map(|y| y.to_string()).collect::<Vec<_>>().join(", ")
    }
}

pub fn fact_changes(before: &Facts, after: &Facts) -> Vec<(String, String)> {
    let mut changes = Vec::new();

    // 1. class_years
    if matches!(before.class_years.value, ClassYears::Unknown) {
        match &after.class_years.value {
            ClassYears::Any => changes.push(("class_years".to_string(), "any".to_string())),
            ClassYears::Grad(years) => {
                changes.push(("class_years".to_string(), format_class_years(years)));
            }
            ClassYears::Unknown => {}
        }
    }

    // 2. gpa
    if matches!(before.gpa.value, Gpa::Unknown) {
        match &after.gpa.value {
            Gpa::Min(val) => changes.push(("gpa".to_string(), format!("min {val}"))),
            Gpa::Asked => changes.push(("gpa".to_string(), "asked".to_string())),
            Gpa::NotMentioned => changes.push(("gpa".to_string(), "not_mentioned".to_string())),
            Gpa::Unknown => {}
        }
    }

    // 3. sponsorship
    if matches!(before.sponsorship.value, Sponsorship::Unknown) {
        match &after.sponsorship.value {
            Sponsorship::Offered => changes.push(("sponsorship".to_string(), "offered".to_string())),
            Sponsorship::NotOffered => changes.push(("sponsorship".to_string(), "not_offered".to_string())),
            Sponsorship::Asked => changes.push(("sponsorship".to_string(), "asked".to_string())),
            Sponsorship::NotMentioned => changes.push(("sponsorship".to_string(), "not_mentioned".to_string())),
            Sponsorship::Unknown => {}
        }
    }

    // 4. citizenship
    if matches!(before.citizenship.value, Citizenship::Unknown) {
        match &after.citizenship.value {
            Citizenship::Required => changes.push(("citizenship".to_string(), "required".to_string())),
            Citizenship::NotMentioned => changes.push(("citizenship".to_string(), "not_mentioned".to_string())),
            Citizenship::Unknown => {}
        }
    }

    // 5. experience
    if matches!(before.experience.value, Experience::Unknown) {
        match &after.experience.value {
            Experience::Required => changes.push(("experience".to_string(), "required".to_string())),
            Experience::Preferred => changes.push(("experience".to_string(), "preferred".to_string())),
            Experience::NotMentioned => changes.push(("experience".to_string(), "not_mentioned".to_string())),
            Experience::Unknown => {}
        }
    }

    changes
}

pub fn classify_429(body: &str) -> RateLimit {
    let lower = body.to_lowercase();

    if lower.contains("per minute")
        || lower.contains("perminute")
        || lower.contains("(tpm)")
        || lower.contains("(rpm)")
        || lower.contains("tokens per minute")
        || lower.contains("requests per minute")
    {
        return RateLimit::PerMinute;
    }

    if !(lower.contains("per day")
        || lower.contains("perday")
        || lower.contains("(tpd)")
        || lower.contains("(rpd)")
        || lower.contains("quota")
        || lower.contains("resource_exhausted"))
    {
        return RateLimit::PerMinute;
    }

    let detail = extract_daily_detail(body);
    RateLimit::Daily(detail)
}

fn extract_daily_detail(body: &str) -> String {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(body) else {
        return String::new();
    };

    if let Some(msg) = val.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
        // anchor on " on tokens"/" on requests": a bare "on " also matches inside "organization `org_x`"
        let start = msg
            .find(" on tokens")
            .or_else(|| msg.find(" on requests"))
            .map(|i| i + 1)
            .unwrap_or(0);
        let slice = &msg[start..];
        if let Some(please_idx) = slice.find("Please try again in ") {
            let after = please_idx + "Please try again in ".len();
            if let Some(dot_idx) = slice[after..].find('.') {
                let end = after + dot_idx;
                return slice[..end].trim().to_string();
            } else {
                return slice.trim().to_string();
            }
        } else if let Some(dot_idx) = slice.find('.') {
            return slice[..dot_idx].trim().to_string();
        } else {
            return slice.trim().to_string();
        }
    }

    if let Some(details) = val.get("error").and_then(|e| e.get("details")).and_then(|d| d.as_array()) {
        let mut quota_id: Option<&str> = None;
        let mut quota_value: Option<&str> = None;
        let mut retry_delay: Option<&str> = None;

        for item in details {
            if let Some(violations) = item.get("violations").and_then(|v| v.as_array()) {
                for v in violations {
                    if let Some(qid) = v.get("quotaId").and_then(|q| q.as_str()) {
                        quota_id = Some(qid);
                    }
                    if let Some(qval) = v.get("quotaValue").and_then(|q| q.as_str()) {
                        quota_value = Some(qval);
                    }
                }
            }
            if let Some(delay) = item.get("retryDelay").and_then(|d| d.as_str()) {
                retry_delay = Some(delay);
            }
        }

        if quota_id.is_some() || quota_value.is_some() {
            let mut parts = String::new();
            if let Some(qid) = quota_id {
                parts.push_str(qid);
            }
            if let Some(qval) = quota_value {
                if !parts.is_empty() {
                    parts.push_str(&format!(", limit {qval}"));
                } else {
                    parts.push_str(&format!("limit {qval}"));
                }
            }

            if let Some(delay_str) = retry_delay {
                if let Some(sec_str) = delay_str.strip_suffix('s') {
                    if let Ok(sec) = sec_str.parse::<f64>() {
                        let resets = if sec >= 3600.0 {
                            let hours = (sec / 3600.0).round() as u64;
                            format!("~{hours}h")
                        } else {
                            let mins = (sec / 60.0).round() as u64;
                            format!("~{mins}m")
                        };
                        parts.push_str(&format!("; resets in {resets}"));
                    }
                }
            }

            return parts;
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FormQuestion;

    #[test]
    fn parses_and_merges_only_unknown() {
        let mut cur = Facts::unknown();
        cur.gpa = Fact {
            value: Gpa::NotMentioned,
            evidence: None,
        };
        let ans = r#"```json
{"sponsorship":{"value":"asked","evidence":"We cannot sponsor H-1B."},"gpa":{"value":3.5,"evidence":"3.5 GPA"},
 "class_years":{"value":[2029,2030],"evidence":"rising juniors and sophomores"},"citizenship":{"value":"required","evidence":""},
 "experience":{"value":"bogus","evidence":"x"}}
```"#;
        let page = PortalPage {
            description: "We cannot sponsor H-1B. 3.5 GPA. rising juniors and sophomores.".into(),
            questions: vec![],
        };
        let f = parse_answer(ans, &cur, "SWE Intern", &page).unwrap();
        assert_eq!(f.sponsorship.value, Sponsorship::Asked);
        assert_eq!(
            f.sponsorship.evidence.as_ref().unwrap().source,
            Source::Llm
        );
        assert_eq!(f.gpa.value, Gpa::NotMentioned); // was not Unknown -> unchanged
        assert_eq!(
            f.class_years.value,
            ClassYears::Grad([2029, 2030].into_iter().collect())
        );
        assert_eq!(f.citizenship.value, Citizenship::Unknown); // empty evidence -> rejected
        assert_eq!(f.experience.value, Experience::Unknown); // invalid value -> stays Unknown
    }

    #[test]
    fn non_json_is_err() {
        let page = PortalPage {
            description: "".into(),
            questions: vec![],
        };
        assert!(parse_answer("sorry, I can't", &Facts::unknown(), "SWE Intern", &page).is_err());
    }

    #[test]
    fn test_classify_429_groq_and_gemini() {
        let groq_tpd = r#"{"error":{"message":"Rate limit reached for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on tokens per day (TPD): Limit 200000, Used 199586, Requested 1033. Please try again in 6h12m. Need more tokens? Upgrade to Dev Tier today at https://console.groq.com/settings/billing","type":"tokens","code":"rate_limit_exceeded"}}"#;
        match classify_429(groq_tpd) {
            RateLimit::Daily(detail) => {
                assert!(detail.contains("Limit 200000"), "expected Limit 200000, got: {detail}");
                assert!(detail.contains("6h12m"), "expected 6h12m, got: {detail}");
                assert!(detail.starts_with("on tokens per day"), "detail must not start inside 'organization': {detail}");
            }
            RateLimit::PerMinute => panic!("expected Daily for Groq TPD"),
        }

        let groq_tpm = r#"{"error":{"message":"Rate limit reached for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on tokens per minute (TPM): Limit 8000, Used 7900, Requested 1033. Please try again in 7.5s. Need more tokens? Upgrade to Dev Tier today at https://console.groq.com/settings/billing","type":"tokens","code":"rate_limit_exceeded"}}"#;
        assert_eq!(classify_429(groq_tpm), RateLimit::PerMinute);

        let gemini_daily = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests","quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier","quotaValue":"20"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"40313s"}]}}"#;
        match classify_429(gemini_daily) {
            RateLimit::Daily(detail) => {
                assert!(detail.contains("limit 20"), "expected limit 20, got: {detail}");
                assert!(detail.contains("~11h"), "expected ~11h, got: {detail}");
            }
            RateLimit::PerMinute => panic!("expected Daily for Gemini daily"),
        }

        let gemini_pm = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests","quotaId":"GenerateRequestsPerMinutePerProjectPerModel-FreeTier","quotaValue":"20"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"21s"}]}}"#;
        assert_eq!(classify_429(gemini_pm), RateLimit::PerMinute);

        assert_eq!(classify_429("slow down"), RateLimit::PerMinute);
    }

    #[test]
    fn test_classify_429_edge_cases() {
        let gemini_no_s = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"x","quotaId":"qid","quotaValue":"10"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"4000"}]}}"#;
        match classify_429(gemini_no_s) {
            RateLimit::Daily(detail) => {
                assert_eq!(detail, "qid, limit 10");
            }
            _ => panic!("expected Daily"),
        }

        let gemini_non_num = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"x","quotaId":"qid","quotaValue":"10"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"foobars"}]}}"#;
        match classify_429(gemini_non_num) {
            RateLimit::Daily(detail) => {
                assert_eq!(detail, "qid, limit 10");
            }
            _ => panic!("expected Daily"),
        }

        let gemini_mins = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"x","quotaId":"qid","quotaValue":"10"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"300s"}]}}"#;
        match classify_429(gemini_mins) {
            RateLimit::Daily(detail) => {
                assert!(detail.contains("~5m"));
            }
            _ => panic!("expected Daily"),
        }

        let raw_daily = "Daily quota exceeded";
        match classify_429(raw_daily) {
            RateLimit::Daily(detail) => {
                assert_eq!(detail, "");
            }
            _ => panic!("expected Daily"),
        }
    }

    #[test]
    fn test_focus_excerpt_sponsorship_and_neighbors() {
        let desc = "Sentence 0 about company.\nSentence 1 about culture.\nSentence 2 about team.\nSentence 3 mentioning visa sponsorship is not offered.\nSentence 4 about benefits.\nSentence 5 about office location.\nSentence 6 about food.";
        let res = focus_excerpt(desc, &["sponsorship"]);
        assert_eq!(
            res,
            "Sentence 2 about team\nSentence 3 mentioning visa sponsorship is not offered\nSentence 4 about benefits"
        );
    }

    #[test]
    fn test_focus_excerpt_opt_word_match() {
        let desc = "We have an option for remote work.\nAnother unrelated sentence.\nYet another sentence.";
        let res = focus_excerpt(desc, &["sponsorship"]);
        assert_eq!(res, desc);

        let desc2 = "CPT/OPT students are welcome to apply.\nWe offer great mentorship.\nApply today.";
        let res2 = focus_excerpt(desc2, &["sponsorship"]);
        assert!(res2.contains("CPT/OPT students are welcome to apply"));
    }

    #[test]
    fn test_focus_excerpt_no_match_returns_first_4000_chars() {
        let long_desc = "Hello world. ".repeat(400);
        let res = focus_excerpt(&long_desc, &["sponsorship"]);
        assert_eq!(res.chars().count(), 4_000);
        assert_eq!(res, truncate_chars(&long_desc, 4_000));
    }

    #[test]
    fn test_focus_excerpt_capped_at_4000_chars() {
        let sentence = "We do not offer visa sponsorship. ";
        let desc = sentence.repeat(200);
        let res = focus_excerpt(&desc, &["sponsorship"]);
        assert_eq!(res.chars().count(), 4_000);
    }

    #[test]
    fn test_focus_excerpt_only_uses_active_unknowns() {
        let desc = "Minimum 3.5 GPA required.\nWe love learning.\nUS citizenship required.";
        let res = focus_excerpt(desc, &["gpa"]);
        assert_eq!(res, "Minimum 3.5 GPA required\nWe love learning");
        assert!(!res.contains("citizenship"));

        let res2 = focus_excerpt(desc, &["citizenship"]);
        assert_eq!(res2, "We love learning\nUS citizenship required.");
        assert!(!res2.contains("GPA"));
    }

    #[test]
    fn prompt_mentions_only_unknown_and_years() {
        let mut f = Facts::unknown();
        f.gpa = Fact {
            value: Gpa::Asked,
            evidence: None,
        };
        let page = PortalPage {
            description: "d".repeat(20_000),
            questions: vec![
                FormQuestion {
                    label: "GPA?".into(),
                    options: vec!["3.5+".into()],
                },
                FormQuestion {
                    label: "Clearance?".into(),
                    options: vec![],
                },
            ],
        };
        let (sys, user) = build_messages("SWE Intern", &page, &f, 2027);
        assert!(sys.contains("summer 2027") && sys.contains("graduates 2030"));
        assert!(sys.contains("Current students") && sys.contains("means \"any\""));
        assert!(user.contains("Facts to settle: class_years, sponsorship, citizenship, experience"));
        assert!(user.contains("- GPA? [options: 3.5+]"));
        assert!(user.contains("- Clearance?\n"));
        assert!(!user.contains("- Clearance? [options:"));
        assert!(user.chars().count() < 13_000);
    }

    #[test]
    fn parse_answer_class_years_any_and_gpa_range() {
        let cur = Facts::unknown();
        let ans = r#"{
            "class_years": {"value": "any", "evidence": "All class years welcome."},
            "gpa": {"value": 4.5, "evidence": "4.5 GPA"},
            "experience": {"value": "preferred", "evidence": "Prior experience is a plus."}
        }"#;
        let page = PortalPage {
            description: "All class years welcome. 4.5 GPA. Prior experience is a plus.".into(),
            questions: vec![],
        };
        let f = parse_answer(ans, &cur, "SWE Intern", &page).unwrap();
        assert_eq!(f.class_years.value, ClassYears::Any);
        assert_eq!(f.class_years.evidence.as_ref().unwrap().source, Source::Llm);
        assert_eq!(f.gpa.value, Gpa::Unknown); // 4.5 is outside [1.0, 4.0]
        assert_eq!(f.experience.value, Experience::Preferred);
    }

    #[test]
    fn parse_answer_code_fence_without_json_tag() {
        let cur = Facts::unknown();
        let ans = "```\n{\"sponsorship\": {\"value\": \"offered\", \"evidence\": \"Visa sponsorship available.\"}}\n```";
        let page = PortalPage {
            description: "Visa sponsorship available.".into(),
            questions: vec![],
        };
        let f = parse_answer(ans, &cur, "SWE Intern", &page).unwrap();
        assert_eq!(f.sponsorship.value, Sponsorship::Offered);
        assert_eq!(f.sponsorship.evidence.as_ref().unwrap().snippet, "Visa sponsorship available.");
    }

    #[test]
    fn evidence_in_posting_cases() {
        let page = PortalPage {
            description: "We will only accept applications from current undergraduate students. Must have 3.0 GPA or higher.".into(),
            questions: vec![
                FormQuestion {
                    label: "Are you authorized to work in the US?".into(),
                    options: vec!["Yes".into(), "No".into()],
                },
            ],
        };

        // - "unknown" -> false; "yes" -> false; "" -> false
        assert!(!evidence_in_posting("unknown", "SWE Intern", &page));
        assert!(!evidence_in_posting("yes", "SWE Intern", &page));
        assert!(!evidence_in_posting("", "SWE Intern", &page));

        // - an exact sentence from the description -> true (also with different case/extra spaces/curly quotes)
        assert!(evidence_in_posting("Must have 3.0 GPA or higher.", "SWE Intern", &page));
        assert!(evidence_in_posting("  must  HAVE   3.0 gpa or higher.  ", "SWE Intern", &page));
        assert!(evidence_in_posting("“Must have 3.0 GPA or higher.”", "SWE Intern", &page));

        // - "We will only accept ... current undergraduate students." when description contains "We will only accept applications from current undergraduate students." -> true
        assert!(evidence_in_posting("We will only accept ... current undergraduate students.", "SWE Intern", &page));
        assert!(evidence_in_posting("We will only accept … current undergraduate students.", "SWE Intern", &page));

        // - a fabricated sentence not in the posting -> false
        assert!(!evidence_in_posting("Only accepting applications from class of 2028.", "SWE Intern", &page));

        // - a quote taken from a form question label -> true
        assert!(evidence_in_posting("Are you authorized to work in the US?", "SWE Intern", &page));
    }

    #[test]
    fn parse_answer_amazon_and_decided_with_real_quote() {
        let cur = Facts::unknown();
        let page = PortalPage {
            description: "This is a software development internship. Must be enrolled in a degree program.".into(),
            questions: vec![],
        };

        // Amazon case: citizenship {"value":"not_mentioned","evidence":"unknown"} -> stays Unknown
        let ans_amazon = r#"{"citizenship": {"value": "not_mentioned", "evidence": "unknown"}}"#;
        let f = parse_answer(ans_amazon, &cur, "SWE Intern", &page).unwrap();
        assert_eq!(f.citizenship.value, Citizenship::Unknown);
        assert!(f.citizenship.evidence.is_none());

        // Decided value with real quote -> accepted with Source::Llm
        let ans_real = r#"{"class_years": {"value": "any", "evidence": "Must be enrolled in a degree program."}}"#;
        let f2 = parse_answer(ans_real, &cur, "SWE Intern", &page).unwrap();
        assert_eq!(f2.class_years.value, ClassYears::Any);
        let ev = f2.class_years.evidence.as_ref().unwrap();
        assert_eq!(ev.source, Source::Llm);
        assert_eq!(ev.snippet, "Must be enrolled in a degree program.");
    }

    #[test]
    fn from_config_missing_or_empty_key() {
        let mut cfg = LlmConfig {
            base_url: "https://example.com/v1".into(),
            model: "model".into(),
            api_key_env: "DEFINITELY_NON_EXISTENT_VAR_XYZ_12345".into(),
            requests_per_minute: 10,
            max_age_days: 30,
            reasoning_effort: None,
        };
        let err = LlmClient::from_config(&cfg).unwrap_err().to_string();
        assert!(err.contains("DEFINITELY_NON_EXISTENT_VAR_XYZ_12345"));

        // If var is set but empty
        unsafe {
            std::env::set_var("DEFINITELY_EMPTY_VAR_XYZ_12345", "");
        }
        cfg.api_key_env = "DEFINITELY_EMPTY_VAR_XYZ_12345".into();
        let err = LlmClient::from_config(&cfg).unwrap_err().to_string();
        assert!(err.contains("DEFINITELY_EMPTY_VAR_XYZ_12345"));
    }

    #[test]
    #[ignore]
    fn live_llm() {
        // needs LLM_API_KEY + a real [llm] config; settle on gh_sevenresearch page -> Facts
    }

    #[test]
    fn test_parse_duration_table() {
        assert_eq!(parse_duration("1.567s"), Some(Duration::from_millis(1567)));
        assert_eq!(parse_duration("7.5s"), Some(Duration::from_millis(7500)));
        assert_eq!(parse_duration("12ms"), Some(Duration::from_millis(12)));
        assert_eq!(parse_duration("6m22.5s"), Some(Duration::from_millis(382_500)));
        assert_eq!(parse_duration("6h12m"), Some(Duration::from_secs(22320)));
        assert_eq!(parse_duration("1h2m3.5s"), Some(Duration::from_millis(3_723_500)));
        assert_eq!(parse_duration("40313s"), Some(Duration::from_secs(40313)));
        assert_eq!(parse_duration("0s"), Some(Duration::ZERO));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("garbage"), None);
        assert_eq!(parse_duration("12m5"), None);
    }

    #[test]
    fn test_pacing_wait_decisions() {
        let now = std::time::Instant::now();

        // 1. rpm spacing
        let state_rpm = PacingState {
            requests_per_minute: 30,
            last_request_at: Some(now),
            remaining_tokens: None,
            reset_tokens: None,
            remaining_requests: None,
            reset_requests: None,
            last_total_tokens: 2000,
        };
        assert_eq!(pacing_wait(&state_rpm, now), Duration::from_millis(2000));

        // 2. low remaining tokens -> waits reset (+200ms)
        let state_tokens = PacingState {
            requests_per_minute: 30,
            last_request_at: Some(now),
            remaining_tokens: Some(500),
            reset_tokens: Some(Duration::from_secs(5)),
            remaining_requests: None,
            reset_requests: None,
            last_total_tokens: 2000,
        };
        assert_eq!(pacing_wait(&state_tokens, now), Duration::from_millis(5200));

        // 3. remaining_requests 0 -> waits reset_requests (+200ms)
        let state_reqs = PacingState {
            requests_per_minute: 30,
            last_request_at: Some(now),
            remaining_tokens: None,
            reset_tokens: None,
            remaining_requests: Some(0),
            reset_requests: Some(Duration::from_secs(3)),
            last_total_tokens: 2000,
        };
        assert_eq!(pacing_wait(&state_reqs, now), Duration::from_millis(3200));

        // 4. nothing known -> only rpm spacing
        let state_none = PacingState {
            requests_per_minute: 30,
            last_request_at: Some(now),
            remaining_tokens: None,
            reset_tokens: None,
            remaining_requests: None,
            reset_requests: None,
            last_total_tokens: 2000,
        };
        assert_eq!(pacing_wait(&state_none, now), Duration::from_millis(2000));
    }

    #[test]
    fn test_build_chat_body_reasoning_effort() {
        let mut cfg = LlmConfig {
            base_url: "https://example.com/v1".into(),
            model: "openai/gpt-oss-120b".into(),
            api_key_env: "LLM_API_KEY".into(),
            requests_per_minute: 30,
            max_age_days: 30,
            reasoning_effort: None,
        };
        let body_none = build_chat_body(&cfg, "sys", "user");
        assert!(body_none.get("reasoning_effort").is_none());

        cfg.reasoning_effort = Some("low".into());
        let body_some = build_chat_body(&cfg, "sys", "user");
        assert_eq!(body_some.get("reasoning_effort").and_then(|v| v.as_str()), Some("low"));
    }

    #[test]
    fn test_per_minute_retry_wait_cases() {
        assert_eq!(per_minute_retry_wait(Some("7"), ""), Duration::from_secs(7));
        assert_eq!(per_minute_retry_wait(None, "Please try again in 7.5s"), Duration::from_millis(7500));
        assert_eq!(per_minute_retry_wait(None, ""), Duration::from_secs(60));
        assert_eq!(per_minute_retry_wait(None, "Please try again in 6h12m"), Duration::from_secs(120));
    }

    #[test]
    fn test_blocking_evidence_cases() {
        // experience required
        let excel_ev = "This position requires advanced Microsoft Excel skills and prior experience using programming or statistical languages (SQL or Python preferred).";
        assert!(!blocking_evidence_ok("experience", "required", excel_ev));
        assert!(blocking_evidence_ok("experience", "required", "Prior internship experience is required."));
        assert!(blocking_evidence_ok("experience", "preferred", excel_ev));

        // citizenship required
        assert!(!blocking_evidence_ok("citizenship", "required", "Are you a U.S. citizen, a lawful permanent resident of the United States, an asylee, a refugee?"));
        assert!(!blocking_evidence_ok("citizenship", "required", "Are you a US citizen? Note: this information is required by federal, state, or local government contracts."));
        assert!(blocking_evidence_ok("citizenship", "required", "Must be a U.S. citizen."));
        assert!(!blocking_evidence_ok("class_years", "[2027,2028]", "Rising Junior, Senior, or Master's student preferred"));
        assert!(!blocking_evidence_ok("class_years", "[2030]", "- 2030 graduation year preferred"));
        assert!(blocking_evidence_ok("class_years", "[2027,2028]", "Open to rising juniors and seniors."));
        assert!(blocking_evidence_ok("citizenship", "required", "Do you currently hold an active US security clearance?"));
        assert!(blocking_evidence_ok("citizenship", "required", "must be a \"U.S. Person\" ... includes U.S. citizens"));
        assert!(!blocking_evidence_ok("citizenship", "required", "United States"));
        assert!(blocking_evidence_ok("citizenship", "not_mentioned", "United States"));

        // sponsorship not_offered
        assert!(blocking_evidence_ok("sponsorship", "not_offered", "Must be legally authorized to work in the United States for any employer without sponsorship"));
        assert!(!blocking_evidence_ok("sponsorship", "not_offered", "We are a great team"));

        // class_years [2028]
        let undergrad_ev = "We will only accept applications from current undergraduate students.";
        assert!(!blocking_evidence_ok("class_years", "[2028]", undergrad_ev));
        assert!(blocking_evidence_ok("class_years", "[2028]", "Class of 2028 only"));
        assert!(blocking_evidence_ok("class_years", "any", undergrad_ev));
    }

    #[test]
    fn huge_dropdowns_are_capped() {
        // real shape: Palantir's university dropdown had 3,302 options (request hit 22K tokens, HTTP 413)
        let unis: Vec<String> = (0..3302).map(|i| format!("University number {i}")).collect();
        let qs = vec![
            FormQuestion { label: "Which university are you currently attending?".into(), options: unis },
            FormQuestion { label: "Will you require sponsorship?".into(), options: vec!["Yes".into(), "No".into()] },
        ];
        let out = format_questions(&qs);
        assert!(out.contains("…(+3287 more)"), "{out}");
        assert!(out.contains("- Will you require sponsorship? [options: Yes | No]"));
        assert!(out.chars().count() <= 3_100);
        // many questions: hard cap on the whole section
        let many: Vec<FormQuestion> = (0..500)
            .map(|i| FormQuestion { label: format!("Question number {i} about something"), options: vec![] })
            .collect();
        let out = format_questions(&many);
        assert!(out.ends_with("- …(more questions omitted)\n"));
        assert!(out.chars().count() <= 3_100);
    }

    #[test]
    fn sponsorship_visa_alone_is_not_a_refusal() {
        // REAL false "no" from Groq: this welcomes F-1 students who already hold a study visa
        let compassion = "Legally eligible for employment in the United States (i.e., if an international student, must already have a work or study visa, SSN, etc.)";
        assert!(!blocking_evidence_ok("sponsorship", "not_offered", compassion));
        assert!(blocking_evidence_ok("sponsorship", "not_offered", "Must be legally authorized to work in the United States for any employer without sponsorship"));
        assert!(blocking_evidence_ok("sponsorship", "not_offered", "CPT/OPT candidates are not eligible."));
        assert!(blocking_evidence_ok("sponsorship", "not_offered", "Open only to U.S. citizens and green card holders."));
    }

    #[test]
    fn test_parse_answer_blocking_evidence_rules() {
        // REAL case: experience "required" with Excel evidence -> stays Unknown
        let excel_desc = "This position requires advanced Microsoft Excel skills and prior experience using programming or statistical languages (SQL or Python preferred).";
        let page_excel = PortalPage {
            description: excel_desc.into(),
            questions: vec![],
        };
        let ans_excel = format!(
            r#"{{"experience": {{"value": "required", "evidence": "{}"}}}}"#,
            excel_desc
        );
        let f_excel = parse_answer(&ans_excel, &Facts::unknown(), "Analyst Intern", &page_excel).unwrap();
        assert_eq!(f_excel.experience.value, Experience::Unknown);
        assert!(f_excel.experience.evidence.is_none());

        // experience "required" with internship evidence -> accepted
        let intern_desc = "Prior internship experience is required.";
        let page_intern = PortalPage {
            description: intern_desc.into(),
            questions: vec![],
        };
        let ans_intern = format!(
            r#"{{"experience": {{"value": "required", "evidence": "{}"}}}}"#,
            intern_desc
        );
        let f_intern = parse_answer(&ans_intern, &Facts::unknown(), "Analyst Intern", &page_intern).unwrap();
        assert_eq!(f_intern.experience.value, Experience::Required);

        // citizenship "required" with "must be a \"U.S. Person\" ... includes U.S. citizens" -> accepted
        let cit_desc = "must be a \"U.S. Person\" which includes U.S. citizens and permanent residents.";
        let page_cit = PortalPage {
            description: cit_desc.into(),
            questions: vec![],
        };
        let ans_cit = r#"{"citizenship": {"value": "required", "evidence": "must be a \"U.S. Person\" ... includes U.S. citizens"}}"#;
        let f_cit = parse_answer(ans_cit, &Facts::unknown(), "Analyst Intern", &page_cit).unwrap();
        assert_eq!(f_cit.citizenship.value, Citizenship::Required);

        // citizenship "required" with "United States" -> rejected
        let us_desc = "Located in the United States.";
        let page_us = PortalPage {
            description: us_desc.into(),
            questions: vec![],
        };
        let ans_us = r#"{"citizenship": {"value": "required", "evidence": "United States"}}"#;
        let f_us = parse_answer(ans_us, &Facts::unknown(), "Analyst Intern", &page_us).unwrap();
        assert_eq!(f_us.citizenship.value, Citizenship::Unknown);

        // sponsorship "not_offered" with authorized without sponsorship -> accepted
        let spon_desc = "Must be legally authorized to work in the United States for any employer without sponsorship.";
        let page_spon = PortalPage {
            description: spon_desc.into(),
            questions: vec![],
        };
        let ans_spon = format!(
            r#"{{"sponsorship": {{"value": "not_offered", "evidence": "{}"}}}}"#,
            spon_desc
        );
        let f_spon = parse_answer(&ans_spon, &Facts::unknown(), "Analyst Intern", &page_spon).unwrap();
        assert_eq!(f_spon.sponsorship.value, Sponsorship::NotOffered);

        // class_years [2028] with "We will only accept applications from current undergraduate students." -> rejected
        let cy_desc = "We will only accept applications from current undergraduate students.";
        let page_cy = PortalPage {
            description: cy_desc.into(),
            questions: vec![],
        };
        let ans_cy = format!(
            r#"{{"class_years": {{"value": [2028], "evidence": "{}"}}}}"#,
            cy_desc
        );
        let f_cy = parse_answer(&ans_cy, &Facts::unknown(), "Analyst Intern", &page_cy).unwrap();
        assert_eq!(f_cy.class_years.value, ClassYears::Unknown);
    }
}
