-- A character's weekly routine and home country (JSON); see models/schedule.rs.
ALTER TABLE Characters ADD COLUMN ScheduleJson TEXT NULL;
