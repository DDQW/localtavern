-- What a roleplay reply did to the in-story clock (the clock before it, and how far it moved), so
-- regenerating or deleting the reply can put the clock back (JSON).
ALTER TABLE ChatMessages ADD COLUMN ClockJson TEXT NULL;
