-- Sampling parameters per prompt preset; NULL means "provider default".
ALTER TABLE PromptPresets ADD COLUMN Temperature REAL;
ALTER TABLE PromptPresets ADD COLUMN TopP REAL;
ALTER TABLE PromptPresets ADD COLUMN TopK INTEGER;
ALTER TABLE PromptPresets ADD COLUMN MaxOutputTokens INTEGER;
