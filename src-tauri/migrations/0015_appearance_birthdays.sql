-- Quick appearance builder picks (JSON), the rendered/edited appearance text, and a birth date
-- (YYYY-MM-DD) used with the in-story clock.
ALTER TABLE Characters ADD COLUMN AppearanceJson TEXT NULL;
ALTER TABLE Characters ADD COLUMN AppearanceText TEXT NOT NULL DEFAULT '';
ALTER TABLE Characters ADD COLUMN BirthDate TEXT NULL;
ALTER TABLE Personas ADD COLUMN AppearanceJson TEXT NULL;
ALTER TABLE Personas ADD COLUMN AppearanceText TEXT NOT NULL DEFAULT '';
ALTER TABLE Personas ADD COLUMN BirthDate TEXT NULL;
