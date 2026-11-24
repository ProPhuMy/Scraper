#![allow(dead_code)]

use regex::Regex;

fn is_skip_element(tag: &str) -> bool {
    matches!(
        tag,
        "script" | "style" | "noscript" | "svg" | "template" | "head"
    )
}

fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "br"
            | "li"
            | "ul"
            | "ol"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "tr"
            | "table"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "blockquote"
            | "pre"
    )
}

struct Walker {
    output: String,
    pending_bullet: bool,
}

type NodeRef<'a> = <scraper::ElementRef<'a> as std::ops::Deref>::Target;

fn walk(node: NodeRef<'_>, walker: &mut Walker) {
    match node.value() {
        scraper::node::Node::Element(elem) => {
            let tag = elem.name();
            if is_skip_element(tag) {
                return;
            }

            let is_block = is_block_element(tag);
            let is_li = tag == "li";

            if is_block {
                walker.output.push('\n');
            }

            let prev_bullet = walker.pending_bullet;
            if is_li {
                walker.pending_bullet = true;
            }

            for child in node.children() {
                walk(child, walker);
            }

            if is_li {
                walker.pending_bullet = prev_bullet;
            }

            if is_block {
                walker.output.push('\n');
            }
        }
        scraper::node::Node::Text(text) => {
            let s: &str = text.as_ref();
            if s.trim().is_empty() {
                walker.output.push_str(s);
            } else {
                if walker.pending_bullet {
                    walker.output.push_str("- ");
                    walker.pending_bullet = false;
                    walker.output.push_str(s.trim_start());
                } else {
                    walker.output.push_str(s);
                }
            }
        }
        _ => {
            for child in node.children() {
                walk(child, walker);
            }
        }
    }
}

pub fn html_to_text(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    let mut walker = Walker {
        output: String::new(),
        pending_bullet: false,
    };

    for child in doc.tree.root().children() {
        walk(child, &mut walker);
    }

    let space_re = Regex::new(r"[ \t]+").expect("valid regex");
    let lines: Vec<String> = walker
        .output
        .lines()
        .map(|line| space_re.replace_all(line, " ").trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    lines.join("\n")
}

pub fn unescape_entities(s: &str) -> String {
    let fragment = scraper::Html::parse_fragment(s);
    fragment.root_element().text().collect()
}

pub fn normalize(s: &str) -> String {
    // 1. Lowercase
    let mut s = s.to_lowercase();

    // 2. Unicode replacements:
    // \u{00a0} -> space, curly quotes, en/em dashes
    s = s.replace('\u{00a0}', " ");
    s = s.replace('’', "'").replace('‘', "'");
    s = s.replace('“', "\"").replace('”', "\"");
    s = s.replace('–', "-").replace('—', "-");

    // 3. u.s.a. before u.s.
    s = s.replace("u.s.a.", "usa");
    s = s.replace("u.s.", "us");

    // 4. opt-in, opt in, opt-out, opt out -> optin / optout
    s = s.replace("opt-in", "optin");
    s = s.replace("opt in", "optin");
    s = s.replace("opt-out", "optout");
    s = s.replace("opt out", "optout");

    // 5. Collapse spaces and tabs. Keep \n.
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let space_re = Regex::new(r"[ \t]+").expect("valid regex");
    let lines: Vec<String> = s
        .split('\n')
        .map(|line| space_re.replace_all(line, " ").trim().to_string())
        .collect();

    lines.join("\n")
}

pub fn sentences(normalized: &str) -> Vec<String> {
    let split_re = Regex::new(r"\r?\n|[.!?;…]\s+").expect("valid regex");
    split_re
        .split(normalized)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn snippet(s: &str) -> String {
    truncate_chars(s.trim(), 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_blocks_become_lines() {
        let t = html_to_text("<div><p>Hello <b>world</b></p><ul><li>3.0 GPA</li><li>US citizen</li></ul><script>x()</script></div>");
        assert_eq!(t, "Hello world\n- 3.0 GPA\n- US citizen");
    }

    #[test]
    fn unescape_then_text() {
        let raw = "&lt;p&gt;Must have &lt;strong&gt;3.0&lt;/strong&gt; GPA&lt;/p&gt;";
        assert_eq!(html_to_text(&unescape_entities(raw)), "Must have 3.0 GPA");
    }

    #[test]
    fn normalize_handles_unicode() {
        assert_eq!(normalize("U.S.\u{00a0}Citizens – Only’s “x”"), "us citizens - only's \"x\"");
    }

    #[test]
    fn normalize_kills_opt_in() {
        assert_eq!(normalize("Opt-in to SMS; OPT out anytime"), "optin to sms; optout anytime");
    }

    #[test]
    fn sentences_keep_decimals() {
        assert_eq!(
            sentences("minimum 3.0 gpa required. must be us citizen\n- cpt/opt welcome"),
            vec!["minimum 3.0 gpa required", "must be us citizen", "- cpt/opt welcome"]
        );
    }

    #[test]
    fn snippet_is_char_safe() {
        let s = "✱".repeat(300);
        assert_eq!(snippet(&s).chars().count(), 200);
    }

    #[test]
    fn li_with_paragraph_bullet_once() {
        let t = html_to_text("<ul><li><p>Must have 3.0 GPA</p></li></ul>");
        assert_eq!(t, "- Must have 3.0 GPA");
    }

    #[test]
    fn br_produces_line_break() {
        let t = html_to_text("<p>Line 1<br>Line 2<br/>Line 3</p>");
        assert_eq!(t, "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn nested_blocks_no_empty_lines() {
        let t = html_to_text("<div><section><article><p>Nested content</p></article></section></div>");
        assert_eq!(t, "Nested content");
    }

    #[test]
    fn whitespace_between_tags_handled() {
        let t = html_to_text("<div>   \n  <p>  First  </p>  \n  <p>  Second  </p>  \n</div>");
        assert_eq!(t, "First\nSecond");
    }

    #[test]
    fn normalize_usa_before_us_and_opt_preservation() {
        assert_eq!(normalize("U.S.A. and U.S. with CPT/OPT and opt for this"), "usa and us with cpt/opt and opt for this");
        assert_eq!(normalize("opt-out, opt out, opt-in, opt in"), "optout, optout, optin, optin");
    }

    #[test]
    fn sentences_keep_various_decimals_and_trailing_punct() {
        let s = sentences("req 3.5/4.0 gpa. must be us citizen; cpt/opt accepted! apply now.");
        assert_eq!(
            s,
            vec![
                "req 3.5/4.0 gpa",
                "must be us citizen",
                "cpt/opt accepted",
                "apply now."
            ]
        );
    }

    #[test]
    fn truncate_chars_emoji_safety() {
        let emoji = "🦀".repeat(50);
        assert_eq!(truncate_chars(&emoji, 10).chars().count(), 10);
    }
}
