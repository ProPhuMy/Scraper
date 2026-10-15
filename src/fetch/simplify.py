from httpx import AsyncClient
import re
from src.models.Job import Job
from urllib import parse
from urllib.parse import urlparse

URL = "https://raw.githubusercontent.com/SimplifyJobs/Summer2027-Internships/dev/.github/scripts/listings.json"
ROLE_PATTERN = re.compile(
    r"\b("
    r"software|swe|backend|frontend|full[- ]?stack|"
    r"machine learning|ml|artificial intelligence|ai|"
    r"data|computer vision|nlp|embedded|robotics"
    r")\b",
    re.IGNORECASE,)
US_STATES = {
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE",
    "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS",
    "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS",
    "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY",
    "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC",
    "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC",
}
HOSTNAMES = {
    'job-boards.greenhouse.io': 'greenhouse',
    'jobs.ashbyhq.com': 'ashby',
    'jobs.lever.co': 'lever'
}

def is_active(job: dict):
    return True if job["active"] == True else False

def is_relevant_title(job: dict):
    return bool(ROLE_PATTERN.search(job["title"]))

def is_valid_degree(job:dict):
    degrees = job["degrees"]
    keyword = "bachelor's"
    return any(keyword == degree.lower() for degree in degrees)

def is_us_location(job:dict):
    locations = job["locations"]
    for location in locations:
        location_lower = location.lower()
        if "united states" in location_lower or "usa" in location_lower:
            return True
        parts = [part.strip() for part in location.split(',')]
        if parts[-1].upper() in US_STATES:
            return True

    return False

async def get_new_jobs():
    async with AsyncClient() as client:
        response = await client.get(url=URL)
        response.raise_for_status()
        return response.json()

def base_check(job: dict):
    if is_active(job) and is_relevant_title(job) and is_valid_degree(job) and is_us_location(job):
        return True
    return False

def initial_filter(response: list[dict]):
    return [jobs for jobs in response if base_check(jobs)]

def getProvider(url :str):
    parsed = urlparse(url=url).hostname
    if parsed is None:
        return None

    ats = HOSTNAMES.get(parsed)

    if ats is None and parsed.endswith('myworkdayjobs.com'):
        ats = 'workday'
    return ats

def getId(url: str, provider):
    parsed = urlparse(url=url).path
    parseded = parsed.rstrip('/').split('/')
    match provider:
        case 'greenhouse':
            return parseded[-1]
        case 'ashby':
            return parseded[-2]
        case 'lever':
            return parseded[-2]
        case 'workday':
            idk = parseded[-1]
            if '_' in idk:
                return idk.rsplit('_', 1)[-1]
    # just wanna test this match/case feature
    return None
            
def normalize(job_list: list[dict]):
    jobs = []
    for job in job_list:
        url = job["url"]
        provider = getProvider(url)

        jobs.append(
            Job(
                simplify_id=job["id"],
                category=job["category"],
                company=job["company_name"],
                title=job["title"],
                provider=provider,
                ats_id=getId(url=url, provider=provider),
                locations=job["locations"],
                url=url,
                date_posted=job["date_posted"],
                date_updated=job["date_updated"]
            )
        )
    return jobs


async def fetch_jobs() -> list[Job]:
    raw = initial_filter(await get_new_jobs())
    return normalize(raw)