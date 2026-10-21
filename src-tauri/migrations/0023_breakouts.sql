-- Private talks: a breakout conversation between the user and one member of a group chat, part of
-- that group conversation (deleted with it). Groups can let characters start one themselves.
ALTER TABLE Conversations ADD COLUMN ParentConversationId TEXT NULL REFERENCES Conversations(Id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS IX_Conversations_ParentConversationId ON Conversations(ParentConversationId);
ALTER TABLE GroupChats ADD COLUMN PrivateTalks INTEGER NOT NULL DEFAULT 1;
