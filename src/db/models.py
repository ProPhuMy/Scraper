from sqlalchemy import ForeignKey, UniqueConstraint
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship
from sqlalchemy.types import ARRAY, String

class Base(DeclarativeBase):
    pass

class Job(Base):
    __tablename__ = "jobs"

    # __table_args__ = (UniqueConstraint("provider", "ats_id"),)
    # maybe will return to this later

    id: Mapped[int] = mapped_column(primary_key=True)
    simplify_id: Mapped[str] = mapped_column(unique=True)

    category: Mapped[str] = mapped_column()
    company: Mapped[str] = mapped_column()
    title: Mapped[str] = mapped_column()
    provider: Mapped[str | None] = mapped_column(nullable=True)
    ats_id: Mapped[str | None] = mapped_column(nullable=True)

    locations: Mapped[list[str]] = mapped_column(ARRAY(String))
    url: Mapped[str] = mapped_column()

    date_posted: Mapped[int] = mapped_column()
    date_updated: Mapped[int] = mapped_column()

    requirements: Mapped[list[Requirement]] = relationship(back_populates='job', cascade= "all, delete-orphan")
    description: Mapped[JobDescription | None] = relationship(back_populates='job', cascade= "all, delete-orphan")
    # eligibility

class Requirement(Base):
    __tablename__ = "requirements"

    id: Mapped[int] = mapped_column(primary_key=True)
    job_id: Mapped[int] = mapped_column(ForeignKey('jobs.id'))

    job: Mapped[Job] = relationship(back_populates='requirements')

class JobDescription(Base):
    __tablename__ = "descriptions"

    id: Mapped[int] = mapped_column(primary_key=True)
    job_id: Mapped[int] = mapped_column(ForeignKey('jobs.id'), unique=True)
    status: Mapped[str] = mapped_column(server_default='pending')

    text: Mapped[str | None] = mapped_column(nullable=True)
    text_hash: Mapped[str | None] = mapped_column(nullable=True)
    fetched_at: Mapped[int | None] = mapped_column(nullable=True)
    job: Mapped[Job] = relationship(back_populates='description') 