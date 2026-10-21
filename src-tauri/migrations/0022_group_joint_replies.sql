-- Lets the model hand a user's message on to another group member, so several can answer it.
ALTER TABLE GroupChats ADD COLUMN JointReplies INTEGER NOT NULL DEFAULT 0;
