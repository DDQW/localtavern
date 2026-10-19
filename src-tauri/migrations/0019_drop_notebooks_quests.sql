-- The character notebook and quest log were removed: drop their preset blocks and tables.
DELETE FROM PromptBlocks WHERE Type IN (12, 13);
DROP TABLE IF EXISTS CharacterNotebookEntries;
DROP TABLE IF EXISTS QuestLogEntries;
