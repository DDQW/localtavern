-- A compacted conversation: the messages up to and including CoversThroughMessageId are replaced
-- in prompts by SummaryText. They stay in ChatMessages, so the chat still shows them.
-- CoversThroughTimestamp is the fallback when that message has since been deleted.
CREATE TABLE ConversationSummaries (
    ConversationId TEXT PRIMARY KEY REFERENCES Conversations(Id) ON DELETE CASCADE,
    SummaryText TEXT NOT NULL,
    CoversThroughMessageId TEXT NOT NULL,
    CoversThroughTimestamp TEXT NOT NULL,
    UpdatedAt TEXT NOT NULL
);
