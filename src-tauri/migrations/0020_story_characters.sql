-- Side characters a story introduces (people who are not on the character card), with the facts the
-- story has established about them. Display-only: never sent to the model.
CREATE TABLE StoryCharacters (
    Id TEXT PRIMARY KEY,
    ConversationId TEXT NOT NULL REFERENCES Conversations(Id) ON DELETE CASCADE,
    Name TEXT NOT NULL,
    Age INTEGER NULL,
    Role TEXT NOT NULL DEFAULT '',
    Appearance TEXT NOT NULL DEFAULT '',
    AdultDetails TEXT NOT NULL DEFAULT '',
    IntroducedAt TEXT NULL,
    IntroducedMessageId TEXT NULL,
    CreatedAt TEXT NOT NULL
);
CREATE INDEX IX_StoryCharacters_Conversation ON StoryCharacters(ConversationId);
