# scraper

A command-line tool that tells you which software internships you're actually eligible for.

Many job postings never say outright whether they take first-years, need a GPA, sponsor visas or want prior experience. The application form often gives it away, with questions like "Will you require sponsorship?", "What's your GPA?" or a graduation-year dropdown. `scraper` reads the form and the job description for each posting and sorts every job into **apply**, **maybe** or **no**, with the reason.

- Job list: [SimplifyJobs Summer Internships](https://github.com/SimplifyJobs) (filtered to US software/ML/data roles).
- Application forms read from Greenhouse, Ashby and Lever. Descriptions read from those plus Workday and ordinary job pages.
- One self-contained binary. Data lives in a local SQLite file.

## Install

Linux, macOS, or Windows (Git Bash):

```sh
curl -fsSL https://raw.githubusercontent.com/ProPhuMy/scratch/main/install.sh | sh
```

The script picks the right build for your system, checks its SHA-256 checksum, and installs it to `~/.local/bin`. Optional environment variables:

| Variable | Default | Effect |
|---|---|---|
| `SCRAPER_VERSION` | latest release | Install a specific tag, e.g. `v0.1.0` |
| `SCRAPER_INSTALL_DIR` | `~/.local/bin` | Install somewhere else |

## Set up

Download the config template and fill in your profile:

```sh
curl -fsSL https://raw.githubusercontent.com/ProPhuMy/scratch/main/scraper.example.toml -o scraper.toml
```

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
scraper list                       # show apply + maybe
scraper list --verdict no          # show only rejections, with reasons
scraper list --year 2              # "what if I were a sophomore?"
scraper list --all --json          # everything, machine-readable
```

`scan` saves after every job, so you can stop it and run it again later. It only fetches what's left. Re-run `sync` from time to time to pick up new postings.

## How verdicts work

For each job, `scraper` extracts five facts: accepted class years, GPA, visa sponsorship, US citizenship or security clearance, and prior experience. It then compares them with your profile:

- **no**: an explicit conflict, for example "must not require sponsorship now or in the future", "active security clearance required", "graduating between Dec 2027 and Jun 2028" when you graduate in 2030, or a stated GPA minimum above yours.
- **maybe**: something is unclear or only asked about (for example a form asks about sponsorship without refusing it, or the page needs JavaScript and couldn't be read).
- **apply**: nothing in the posting blocks you.

The rules lean toward **maybe** when unsure, because wrongly hiding a job you could have gotten is worse than showing one to double-check.

### Optional: LLM pass for unclear wording

Facts the rules can't settle can be sent to any OpenAI-compatible chat API (Gemini, Groq, OpenRouter, ...). Add this to `scraper.toml`:

```toml
[llm]
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
model = "<model id>"
api_key_env = "LLM_API_KEY"   # read from the environment or a .env file
requests_per_minute = 10
```

The LLM is only asked about facts that are still unknown. Answers must quote the posting, or they're discarded. If you hit a daily quota, `scan` stops cleanly and continues next time.

## Limitations

- Pages that render entirely with JavaScript (for example UltiPro/UKG and Oracle Cloud job boards) can't be read and show up as **maybe**.
- Workday application forms sit behind a login, so for Workday jobs only the description is used.
- Pattern rules can still misread unusual wording. The reason column always quotes the sentence it used, so check it before you skip a job.

## Build from source

Requires Rust (stable) and a C compiler (for the bundled SQLite).

```sh
cargo build --release
cargo test
```

Releases are built by `.github/workflows/release.yml` whenever a `v*` tag is pushed.
