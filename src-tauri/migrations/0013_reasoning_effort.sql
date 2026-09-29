-- ReasoningEffort enum's raw int value; NULL leaves thinking effort to the model's own default,
-- matching the sampling columns' "provider default" convention.
ALTER TABLE PromptPresets ADD COLUMN ReasoningEffort INTEGER;
