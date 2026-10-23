---
name: database-migrate
description: Plan a reversible database schema migration and verify it against a representative backup.
requires_tools:
  - database_read
  - database_write
---
# Database migration

Inspect the current schema and migration history first. Write a forward migration and a tested
rollback, check locking and data conversion costs, and verify both directions against a disposable
copy before proposing application to a live database.
