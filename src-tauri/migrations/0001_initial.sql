CREATE TABLE IF NOT EXISTS Characters (
    Id TEXT PRIMARY KEY,
    Name TEXT NOT NULL,
    AvatarPath TEXT NULL,
    Description TEXT NOT NULL DEFAULT '',
    Personality TEXT NOT NULL DEFAULT '',
    Scenario TEXT NOT NULL DEFAULT '',
    FirstMessagesJson TEXT NOT NULL DEFAULT '[]',
    ExampleDialogue TEXT NOT NULL DEFAULT '',
    TagsJson TEXT NOT NULL DEFAULT '[]',
    RawImportJson TEXT NULL,
    CreatedAt TEXT NOT NULL,
    UpdatedAt TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS Personas (
    Id TEXT PRIMARY KEY,
    Name TEXT NOT NULL,
    AvatarPath TEXT NULL,
    Description TEXT NOT NULL DEFAULT '',
    IsDefault INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS Lorebooks (
    Id TEXT PRIMARY KEY,
    Name TEXT NOT NULL,
    ScanDepth INTEGER NOT NULL DEFAULT 4,
    TokenBudget INTEGER NOT NULL DEFAULT 1000
);

CREATE TABLE IF NOT EXISTS CharacterLorebooks (
    CharacterId TEXT NOT NULL REFERENCES Characters(Id) ON DELETE CASCADE,
    LorebookId TEXT NOT NULL REFERENCES Lorebooks(Id) ON DELETE CASCADE,
    PRIMARY KEY (CharacterId, LorebookId)
);

CREATE TABLE IF NOT EXISTS WorldInfoEntries (
    Id TEXT PRIMARY KEY,
    LorebookId TEXT NOT NULL REFERENCES Lorebooks(Id) ON DELETE CASCADE,
    KeysJson TEXT NOT NULL DEFAULT '[]',
    SecondaryKeysJson TEXT NULL,
    Content TEXT NOT NULL DEFAULT '',
    CaseSensitive INTEGER NOT NULL DEFAULT 0,
    Enabled INTEGER NOT NULL DEFAULT 1,
    InsertionOrder INTEGER NOT NULL DEFAULT 0,
    Position INTEGER NOT NULL DEFAULT 0,
    Depth INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS IX_WorldInfoEntries_LorebookId ON WorldInfoEntries(LorebookId);

CREATE TABLE IF NOT EXISTS GroupChats (
    Id TEXT PRIMARY KEY,
    Name TEXT NOT NULL,
    ActivationMode INTEGER NOT NULL DEFAULT 0,
    AllowSelfResponses INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS GroupChatMembers (
    GroupChatId TEXT NOT NULL REFERENCES GroupChats(Id) ON DELETE CASCADE,
    CharacterId TEXT NOT NULL REFERENCES Characters(Id) ON DELETE CASCADE,
    OrderIndex INTEGER NOT NULL,
    PRIMARY KEY (GroupChatId, CharacterId)
);

CREATE TABLE IF NOT EXISTS PromptPresets (
    Id TEXT PRIMARY KEY,
    Name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS PromptBlocks (
    Id TEXT PRIMARY KEY,
    PresetId TEXT NOT NULL REFERENCES PromptPresets(Id) ON DELETE CASCADE,
    Type INTEGER NOT NULL,
    Enabled INTEGER NOT NULL DEFAULT 1,
    OrderIndex INTEGER NOT NULL,
    CustomText TEXT NULL,
    InjectionDepth INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS IX_PromptBlocks_PresetId ON PromptBlocks(PresetId);

CREATE TABLE IF NOT EXISTS Conversations (
    Id TEXT PRIMARY KEY,
    Title TEXT NOT NULL DEFAULT '',
    Mode INTEGER NOT NULL DEFAULT 0,
    CharacterId TEXT NULL REFERENCES Characters(Id) ON DELETE SET NULL,
    GroupChatId TEXT NULL REFERENCES GroupChats(Id) ON DELETE SET NULL,
    PersonaId TEXT NULL REFERENCES Personas(Id) ON DELETE SET NULL,
    PromptPresetId TEXT NULL REFERENCES PromptPresets(Id) ON DELETE SET NULL,
    ProviderId TEXT NOT NULL DEFAULT 'gemini',
    Model TEXT NOT NULL DEFAULT '',
    AssistantSystemPrompt TEXT NULL,
    CreatedAt TEXT NOT NULL,
    UpdatedAt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS IX_Conversations_UpdatedAt ON Conversations(UpdatedAt);

CREATE TABLE IF NOT EXISTS ChatMessages (
    RowId INTEGER PRIMARY KEY AUTOINCREMENT,
    Id TEXT NOT NULL UNIQUE,
    ConversationId TEXT NOT NULL REFERENCES Conversations(Id) ON DELETE CASCADE,
    Role INTEGER NOT NULL,
    SpeakerCharacterId TEXT NULL,
    Text TEXT NOT NULL DEFAULT '',
    Timestamp TEXT NOT NULL,
    PromptTokens INTEGER NULL,
    CompletionTokens INTEGER NULL,
    TotalTokens INTEGER NULL
);
CREATE INDEX IF NOT EXISTS IX_ChatMessages_ConversationId ON ChatMessages(ConversationId);

CREATE VIRTUAL TABLE IF NOT EXISTS ChatMessagesFts USING fts5(
    Text,
    content='ChatMessages',
    content_rowid='RowId'
);

CREATE TRIGGER IF NOT EXISTS ChatMessages_ai AFTER INSERT ON ChatMessages BEGIN
    INSERT INTO ChatMessagesFts(rowid, Text) VALUES (new.RowId, new.Text);
END;

CREATE TRIGGER IF NOT EXISTS ChatMessages_ad AFTER DELETE ON ChatMessages BEGIN
    INSERT INTO ChatMessagesFts(ChatMessagesFts, rowid, Text) VALUES ('delete', old.RowId, old.Text);
END;

CREATE TRIGGER IF NOT EXISTS ChatMessages_au AFTER UPDATE ON ChatMessages BEGIN
    INSERT INTO ChatMessagesFts(ChatMessagesFts, rowid, Text) VALUES ('delete', old.RowId, old.Text);
    INSERT INTO ChatMessagesFts(rowid, Text) VALUES (new.RowId, new.Text);
END;
