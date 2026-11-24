import asyncio 
import httpx
from src.db.database import Session
from sqlalchemy import select
from src.db.models import Job as DBJob, JobDescription
from urllib.parse import urlparse

async def worker(queue: asyncio.Queue, name, client):
    while True:
        job_id, url, attempt = await queue.get()
        try:
            if attempt < 3:
                result = await client.get(url)
                parsed = urlparse(url=url).hostname
                if result.status_code == 200:
                    print(f"{name}: job={job_id} status={result.status_code} host={parsed} attempt: {attempt} result: {len(result.text)}")
                elif result.status_code in (404, 403):
                    print(f"{name}: job={job_id} status={result.status_code} host={parsed} attempt: {attempt}")
                else:
                    task = (job_id, url, attempt + 1)
                    queue.put_nowait(task)
            else:
                pass
            #logic later
        except httpx.RequestError:
            print(f"{name}: job={job_id} host={parsed} attempt: {attempt}")
            task = (job_id, url, attempt + 1)
            queue.put_nowait(task)
        except Exception as e:
            print(e)
        finally:
            queue.task_done()

async def scan():
    queue = asyncio.Queue()
    async with Session() as session:
        stmt = select(DBJob.id, DBJob.url).join(JobDescription, JobDescription.job_id == DBJob.id).where(JobDescription.status == 'pending').limit(20)
        results = await session.execute(stmt)
        new_results = [(row.id, row.url, 0) for row in results.all()]
        for data in new_results:
            queue.put_nowait(data)
    async with httpx.AsyncClient(follow_redirects=True, timeout=3) as client:
        workers = [asyncio.create_task(worker(queue, f"W{i}", client)) for i in range(10)]
        await queue.join()
        for w in workers:
            w.cancel()
