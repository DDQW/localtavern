-- A group's own story: a scenario for everyone in it (in place of the members' own), and where and
-- when its new chats are set (JSON: country, region, start date and time).
ALTER TABLE GroupChats ADD COLUMN Scenario TEXT NOT NULL DEFAULT '';
ALTER TABLE GroupChats ADD COLUMN SettingJson TEXT NULL;
