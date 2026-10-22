-- The prompt each reply was generated from, kept apart from ChatMessages: every load of a chat
-- read all of them (RawPromptText plus a copy per alternate inside AlternatesJson), and with the
-- whole history in each prompt they grew with the square of the chat's length.
CREATE TABLE MessagePrompts (
    MessageId TEXT NOT NULL REFERENCES ChatMessages(Id) ON DELETE CASCADE,
    AlternateIndex INTEGER NOT NULL,
    Prompt TEXT NOT NULL,
    -- A rowid table on purpose: its key index stays small, so "is there a prompt" and the lookups
    -- never read the prompts themselves (WITHOUT ROWID keeps them in the key's B-tree).
    PRIMARY KEY (MessageId, AlternateIndex)
);

INSERT OR REPLACE INTO MessagePrompts (MessageId, AlternateIndex, Prompt)
SELECT m.Id, CAST(a.key AS INTEGER), json_extract(a.value, '$.RawPrompt')
FROM ChatMessages m, json_each(m.AlternatesJson) a
WHERE json_valid(m.AlternatesJson) AND json_type(m.AlternatesJson) = 'array'
  AND COALESCE(json_extract(a.value, '$.RawPrompt'), '') <> '';

-- A reply from before alternates, or one whose alternate lost its copy.
INSERT OR IGNORE INTO MessagePrompts (MessageId, AlternateIndex, Prompt)
SELECT Id, MAX(ActiveAlternateIndex, 0), RawPromptText
FROM ChatMessages
WHERE Role <> 4 AND COALESCE(RawPromptText, '') <> '';

-- Pictures (Role 4) keep their draft in RawPromptText: the chat needs it for "Again".
UPDATE ChatMessages SET RawPromptText = NULL WHERE Role <> 4 AND RawPromptText IS NOT NULL;

UPDATE ChatMessages
SET AlternatesJson = (
    SELECT json_group_array(json(v))
    FROM (SELECT json_remove(a.value, '$.RawPrompt') AS v FROM json_each(ChatMessages.AlternatesJson) a ORDER BY a.key)
)
WHERE json_valid(AlternatesJson) AND json_type(AlternatesJson) = 'array' AND AlternatesJson LIKE '%"RawPrompt"%';

-- Only a text change needs the search index touched (attachment and prompt writes don't).
DROP TRIGGER IF EXISTS ChatMessages_au;
CREATE TRIGGER ChatMessages_au AFTER UPDATE OF Text ON ChatMessages BEGIN
    INSERT INTO ChatMessagesFts(ChatMessagesFts, rowid, Text) VALUES ('delete', old.RowId, old.Text);
    INSERT INTO ChatMessagesFts(rowid, Text) VALUES (new.RowId, new.Text);
END;
