-- Scheduled pipelines: cron expression (UTC) and scheduler bookkeeping.
ALTER TABLE jobs ADD COLUMN schedule TEXT;
ALTER TABLE jobs ADD COLUMN last_enqueued_at TEXT;
