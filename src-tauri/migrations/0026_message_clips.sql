-- Sounds and spoken lines a reply carries (JSON): each is a play button whose audio is made the
-- first time it is pressed.
ALTER TABLE ChatMessages ADD COLUMN ClipsJson TEXT NULL;
