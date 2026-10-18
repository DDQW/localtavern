-- The appearance paragraph now lives inside the description (one field instead of two). Existing
-- appearance text is appended to the description after a blank line - unless the description
-- already contains it (it was sent twice before) - then the column goes.
UPDATE Characters
SET Description = CASE WHEN trim(Description) = '' THEN trim(AppearanceText)
                       ELSE rtrim(Description) || char(10) || char(10) || trim(AppearanceText) END
WHERE trim(AppearanceText) <> '' AND instr(Description, trim(AppearanceText)) = 0;
ALTER TABLE Characters DROP COLUMN AppearanceText;

UPDATE Personas
SET Description = CASE WHEN trim(Description) = '' THEN trim(AppearanceText)
                       ELSE rtrim(Description) || char(10) || char(10) || trim(AppearanceText) END
WHERE trim(AppearanceText) <> '' AND instr(Description, trim(AppearanceText)) = 0;
ALTER TABLE Personas DROP COLUMN AppearanceText;
