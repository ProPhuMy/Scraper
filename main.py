from src.db.database import initDB, sync_jobs
from src.fetch.simplify import fetch_jobs
import asyncio

async def main():
    await initDB()
    jobs = await fetch_jobs()
    postings = await sync_jobs(jobs_list=jobs)
    print(postings)

if __name__ == "__main__":
    asyncio.run(main(), loop_factory=asyncio.SelectorEventLoop)
