# Scraper

A command-line tool that tells you which software internships you're actually eligible for.

Many job postings never say outright whether they take first-years, need a GPA, sponsor visas or want prior experience. The application form often gives it away, with questions like "Will you require sponsorship?", "What's your GPA?" or a graduation-year dropdown. `scraper` reads the form and the job description for each posting and sorts every job into **apply**, **maybe** or **no**, with the reason.

- Job list: [SimplifyJobs Summer Internships](https://github.com/SimplifyJobs) (filtered to US software/ML/data roles).
- Application forms read from Greenhouse, Ashby, Lever, Workable and BambooHR. Descriptions read from those plus Workday, Oracle Cloud, UltiPro/UKG, and ordinary job pages (including the job data that many JavaScript sites embed).
- One self-contained binary. Data lives in a local SQLite file.

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/ProPhuMy/Scraper/main/install.ps1 | iex
```

**Linux / macOS** (or Git Bash on Windows):

```sh
curl -fsSL https://raw.githubusercontent.com/ProPhuMy/Scraper/main/install.sh | sh
```

Both scripts download the right prebuilt binary for your system (no Rust or source code needed). They verify its SHA-256 checksum and install it to `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows). The PowerShell installer also adds that folder to your user PATH, so open a new terminal afterwards. Optional environment variables:

| Variable | Default | Effect |
|---|---|---|
| `SCRAPER_VERSION` | latest release | Install a specific tag, e.g. `v0.2.1` |
| `SCRAPER_INSTALL_DIR` | `~/.local/bin` | Install somewhere else |
| `SCRAPER_NO_MODIFY_PATH` | unset | `1` = don't touch PATH (PowerShell installer) |

## Set up

Create your config folder, then fill in your profile:

```sh
scraper init
```

This creates `scraper.toml` and a starter `.env` (for the optional LLM key) in:

| OS | Folder |
|---|---|
| Windows | `%APPDATA%\scraper\` |
| Linux / macOS | `$XDG_CONFIG_HOME/scraper/` (default `~/.config/scraper/`) |

It never overwrites files that already exist. The job database (`scraper.db`) lives in the same folder, so `scraper` works from any directory.

Where `scraper` looks for its config:
1. The path passed with `--config`.
2. Otherwise, a `scraper.toml` in the current folder, if there is one.
3. Otherwise, the folder above.

`--db` overrides the database location. Edit the `[profile]` section:

```toml
season_year = 2027            # which Simplify list to use (Summer 2027)

[profile]
class_year = 1                # current year in school, 1-4
# expected_graduation = 2030  # optional; otherwise derived from class_year
# gpa = 3.5                   # leave out if you have no GPA yet
needs_sponsorship = true
has_prior_internship = false
```

Changing the profile takes effect immediately. Verdicts are computed when you run `list`, so you never need to re-scan.

## Use

```sh
scraper sync                       # download and filter the job list
scraper scan                       # read each job's form + description (resumable)
scraper browse                     # interactive browser (below)
scraper list                       # show apply + maybe as a table
scraper list --verdict no          # show only rejections, with reasons
scraper list --year 2              # "what if I were a sophomore?"
scraper list --starred             # also --applied, --new
scraper list --all --json          # everything, machine-readable
```

`scan` saves after every job, so you can stop it and run it again later. It only fetches what's left. Re-run `sync` from time to time to pick up new postings, or press `r` inside `browse`.

### `scraper browse`

A terminal UI with tabs for **Jobs**, **Starred**, **Applied** and **Hidden**, plus a detail panel that quotes the sentence behind every verdict.

| Key | Action |
|---|---|
| `↑` `↓` / `j` `k`, `PgUp` `PgDn`, `g` `G` | move |
| `Tab` / `Shift+Tab` | switch tab |
| `/` | search company, title or location |
| `1` `2` `3` `0` | show apply / maybe / no / all (Jobs tab) |
| `y` | try another class year |
| `Enter` / `→` `←` | expand or collapse a `×N` group |
| `s` / `h` / `a` | star, hide, mark applied (press again to undo) |
| `o` | open the job page in your browser |
| `A` | open the job page **and** mark it applied |
| `r` | refresh: sync + scan in the background with live progress |
| `?` / `q` | help / quit |

- **Grouping:** the same job posted in several cities shows as one `×N` row. Expand it to pick a location. Duplicate links for one location merge into a single row, e.g. `Salt Lake City, UT (2 links)`.
- **Marks:** your marks are saved immediately and survive re-syncs. Applied and starred jobs stay in their tabs even after the posting closes.
- **Badges** in the MARK column: `★` starred, `✓` applied, `N` new since the last sync, `✗` closed (Simplify dropped it, or the page is gone).
- **LLM column:** shows a second opinion (`apply`, `maybe` or `no`, dimmed) when the LLM was asked to clarify vague wording, leaving the main VERDICT rules-only.

### Re-checking old postings

Each `scan` (and `r`) also re-fetches up to 200 jobs that haven't been checked in 7 days, starred and applied first. It catches postings that closed and requirements that changed. A temporarily unreachable page never overwrites results you already have. Tune it in `scraper.toml`:

```toml
[scan]
recheck_days = 7     # 0 turns re-checking off
recheck_max = 200
```

## How verdicts work

For each job, `scraper` extracts five facts: accepted class years, GPA, visa sponsorship, US citizenship or security clearance, and prior experience. It then compares them with your profile:

- **no**: an explicit conflict, for example "must not require sponsorship now or in the future", "active security clearance required", "graduating between Dec 2027 and Jun 2028" when you graduate in 2030, or a stated GPA minimum above yours.
- **maybe**: something is unclear or only asked about (for example a form asks about sponsorship without refusing it, or the page needs JavaScript and couldn't be read).
- **apply**: nothing in the posting blocks you.

The rules lean toward **maybe** when unsure, because wrongly hiding a job you could have gotten is worse than showing one to double-check.

### Optional: LLM pass for unclear wording

Facts the rules can't settle can be sent to any OpenAI-compatible chat API (Gemini, Groq, OpenRouter, ...). Uncomment `LLM_API_KEY` in the `.env` that `scraper init` created, paste your key, and add this to `scraper.toml`:

```toml
[llm]
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
model = "<model id>"
api_key_env = "LLM_API_KEY"   # read from the environment or a .env file
requests_per_minute = 10
```

The LLM is only asked about facts that are still unknown. Answers must quote the posting, or they're discarded. If you hit a daily quota, `scan` stops cleanly and continues next time.

## Limitations

- Sites that block automated requests (iCIMS-hosted boards, Tesla, Meta) and pages that render entirely with JavaScript can't be read and show up as **maybe**.
- Workday application forms sit behind a login, so for Workday jobs only the description is used.
- Pattern rules can still misread unusual wording. The reason column always quotes the sentence it used, so check it before you skip a job.

## Build from source

Requires Rust (stable) and a C compiler (for the bundled SQLite).

```sh
cargo build --release
cargo test
```

Releases are built by `.github/workflows/release.yml` whenever a `v*` tag is pushed.
