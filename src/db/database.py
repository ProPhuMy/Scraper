import os
import re
from dotenv import load_dotenv
from src.models.Job import Job
from src.db.models import Job as DBJob, Base, JobDescription
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.ext.asyncio import create_async_engine, async_sessionmaker

load_dotenv()

engine = create_async_engine(re.sub(r'^postgresql:', 'postgresql+psycopg:', os.getenv('DATABASE_URL')), echo=False)
Session = async_sessionmaker(bind=engine, expire_on_commit=False)

async def initDB():
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

async def sync_jobs(jobs_list: list[Job]) -> list[DBJob]:
    if not jobs_list:
        return []
    
    db_jobs = [job.model_dump() for job in jobs_list]

    stmt = insert(DBJob).values(db_jobs)
    stmt = stmt.on_conflict_do_update(
        index_elements=[DBJob.simplify_id], 
        set_={
            "category": stmt.excluded.category,
            "company": stmt.excluded.company,
            "title": stmt.excluded.title,
            "provider": stmt.excluded.provider,
            "ats_id": stmt.excluded.ats_id,
            "locations":stmt.excluded.locations,
            "url": stmt.excluded.url,
            "date_posted": stmt.excluded.date_posted,
            "date_updated": stmt.excluded.date_updated
            },
        where=stmt.excluded.date_updated > DBJob.date_updated
    ).returning(DBJob)
    
    async with Session() as session:
        result = await session.execute(stmt)
        rows = result.scalars().all()
        if rows:
            stmt = insert(JobDescription).values([{"job_id": job.id} for job in rows])
            stmt = stmt.on_conflict_do_update(
                index_elements=[JobDescription.job_id],
                set_={"status": 'pending'}
            )
            await session.execute(stmt)
        await session.commit()
    return rows
    