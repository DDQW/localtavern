ALTER TABLE Conversations ADD COLUMN WorldStateJson TEXT NULL;

CREATE TABLE CharacterNotebookEntries (
    Id TEXT PRIMARY KEY,
    ConversationId TEXT NOT NULL,
    CharacterId TEXT NOT NULL,
    Content TEXT NOT NULL,
    CreatedAt TEXT NOT NULL
);

CREATE TABLE QuestLogEntries (
    Id TEXT PRIMARY KEY,
    ConversationId TEXT NOT NULL,
    Title TEXT NOT NULL,
    Description TEXT NOT NULL,
    Status INTEGER NOT NULL,
    CreatedAt TEXT NOT NULL,
    UpdatedAt TEXT NOT NULL
);
