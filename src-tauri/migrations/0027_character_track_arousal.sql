-- Whether a roleplay's arousal meters include this character. Existing characters stay included,
-- which is how tracking already worked whenever the story had meters switched on.
ALTER TABLE Characters ADD COLUMN TrackArousal INTEGER NOT NULL DEFAULT 1;
