-- Approximate token budget for how much chat history to send per request; NULL means "unlimited"
-- (send everything), matching the sampling columns' "provider/no limit default" convention.
ALTER TABLE PromptPresets ADD COLUMN MaxContextTokens INTEGER;
