from pydantic import BaseModel

class Job(BaseModel):
    simplify_id: str 

    category: str
    company: str
    title: str
    provider: str | None
    ats_id: str | None

    locations: list[str]
    url: str

    date_posted: int
    date_updated: int