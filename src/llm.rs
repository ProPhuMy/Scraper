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

pub struct LlmClient {
    client: reqwest::Client,
    cfg: LlmConfig,
    api_key: String,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("cfg", &self.cfg)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmOutcome {
    Facts(Facts),
    QuotaExhausted,
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
        Ok(LlmClient {
            client,
            cfg: cfg.clone(),
            api_key: key,
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
        let body = serde_json::json!({
            "model": self.cfg.model,
            "temperature": 0,
            "response_format": {"type": "json_object"},
            "messages": [
                {"role": "system", "content": sys},
                {"role": "user", "content": user}
            ]
        });

        let rpm = self.cfg.requests_per_minute.max(1);
        let pacing_ms = 60_000 / rpm as u64;

        let mut attempt = 0;
        loop {
            attempt += 1;
            tokio::time::sleep(Duration::from_millis(pacing_ms)).await;

            let req = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();

                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        anyhow::bail!("LLM rejected the API key");
                    }

                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if is_quota_error(status.as_u16(), &text) {
                            return Ok(LlmOutcome::QuotaExhausted);
                        }
                        if attempt == 1 {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            continue;
                        } else {
                            anyhow::bail!("HTTP 429 after retry: {text}");
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

                    let settled = parse_answer(content, facts)?;
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
- class_years: \"any\", \"unknown\", or an array of accepted graduation years (integers). The internship is in summer {S}. A current freshman graduates {S+3}, sophomore {S+2}, junior {S+1}.
- gpa: a number (the required minimum), \"asked\", \"not_mentioned\", or \"unknown\".
- sponsorship: \"not_offered\" ONLY if the posting refuses CPT/OPT, is limited to US citizens/permanent residents, or requires no sponsorship now or in the future. Plain \"we do not sponsor visas\" or a question asking about sponsorship is \"asked\". \"offered\" if sponsorship or CPT/OPT is explicitly welcome. Else \"not_mentioned\" or \"unknown\".
- citizenship: \"required\" if US citizenship, US person status, or a security clearance is required or asked for. Else \"not_mentioned\" or \"unknown\". Ignore equal-opportunity boilerplate and \"authorized to work in the US\" questions.
- experience: \"required\" if prior internship/work experience is required, \"preferred\" if it is a plus. Skills like \"experience with Python\" do not count. Else \"not_mentioned\" or \"unknown\".";

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
    for q in &page.questions {
        if q.options.is_empty() {
            user.push_str(&format!("- {}\n", q.label));
        } else {
            user.push_str(&format!("- {} [options: {}]\n", q.label, q.options.join(" | ")));
        }
    }
    user.push_str(&format!(
        "Description:\n{}",
        truncate_chars(&page.description, 12_000)
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

pub fn parse_answer(content: &str, current: &Facts) -> anyhow::Result<Facts> {
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
                if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                if let Some(ev) = get_evidence(val_entry) {
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
                if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                }
                Some("offered") => {
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                }
                Some("not_mentioned") => {
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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
                }
                Some("preferred") => {
                    if let Some(ev) = get_evidence(val_entry) {
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
                    if let Some(ev) = get_evidence(val_entry) {
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

pub fn is_quota_error(status: u16, body: &str) -> bool {
    if status != 429 {
        return false;
    }
    let lower = body.to_lowercase();
    lower.contains("quota") || lower.contains("resource_exhausted") || lower.contains("per day")
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
        let f = parse_answer(ans, &cur).unwrap();
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
        assert!(parse_answer("sorry, I can't", &Facts::unknown()).is_err());
    }

    #[test]
    fn quota_detection() {
        assert!(is_quota_error(
            429,
            r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#
        ));
        assert!(is_quota_error(429, "You have exceeded your quota."));
        assert!(is_quota_error(429, "Limit 100 requests per day exceeded"));
        assert!(!is_quota_error(429, "slow down"));
        assert!(!is_quota_error(500, "quota"));
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
        let f = parse_answer(ans, &cur).unwrap();
        assert_eq!(f.class_years.value, ClassYears::Any);
        assert_eq!(f.class_years.evidence.as_ref().unwrap().source, Source::Llm);
        assert_eq!(f.gpa.value, Gpa::Unknown); // 4.5 is outside [1.0, 4.0]
        assert_eq!(f.experience.value, Experience::Preferred);
    }

    #[test]
    fn parse_answer_code_fence_without_json_tag() {
        let cur = Facts::unknown();
        let ans = "```\n{\"sponsorship\": {\"value\": \"offered\", \"evidence\": \"Visa sponsorship available.\"}}\n```";
        let f = parse_answer(ans, &cur).unwrap();
        assert_eq!(f.sponsorship.value, Sponsorship::Offered);
        assert_eq!(f.sponsorship.evidence.as_ref().unwrap().snippet, "Visa sponsorship available.");
    }

    #[test]
    fn from_config_missing_or_empty_key() {
        let mut cfg = LlmConfig {
            base_url: "https://example.com/v1".into(),
            model: "model".into(),
            api_key_env: "DEFINITELY_NON_EXISTENT_VAR_XYZ_12345".into(),
            requests_per_minute: 10,
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
}
