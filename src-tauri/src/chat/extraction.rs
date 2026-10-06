//! Best-effort background enrichment after a reply: pulling durable facts about the user out of
//! Assistant chats, and keeping a roleplay's character notebooks and quest log up to date. Each one
//! asks the model a single question about one exchange and applies the answer. They run detached
//! from the reply (see `ChatEngine::spawn_enrichment`), and a failure here never reaches the chat -
//! the visible reply already succeeded.

use std::collections::HashMap;

use uuid::Uuid;

use crate::db::Db;
use crate::error::{Error, Result};
use crate::llm::{LlmMessage, LlmRequest, LlmRole};
use crate::models::{Character, CharacterNotebookEntry, QuestLogEntry, QuestStatus, UserMemory};
use crate::prompting::{notebook, quest_log};
use crate::providers::LlmProviderRegistry;
use crate::repo::{notebook as notebook_repo, quest_log as quest_repo, user_memories};

/// Once a character's notebook holds more than this many entries it is condensed back down: often
/// enough that a long campaign stays bounded, rarely enough that it isn't a call every few messages.
pub const CONSOLIDATE_ABOVE_COUNT: usize = 20;

/// Where to send the question and what to store the answer in.
pub struct Extractor<'a> {
    pub db: &'a Db,
    pub providers: &'a LlmProviderRegistry,
    pub provider_id: &'a str,
    pub model: &'a str,
}

/// One exchange: the user's message and the reply to it, each with who wrote it (the names only
/// matter to the roleplay questions; the Assistant memory question labels them User/Assistant).
pub struct Exchange<'a> {
    pub user_name: &'a str,
    pub user_text: &'a str,
    pub reply_name: &'a str,
    pub reply_text: &'a str,
}

impl Exchange<'_> {
    /// The exchange for a roleplay question, each message under its author's name. Unlabelled, a
    /// "*I draw my sword*" from the player is easily credited to a character in the notes.
    fn roleplay_transcript(&self) -> String {
        let (user, reply) = (self.user_name, self.reply_name);
        format!(
            "Exchange (everything in a message, *actions* included, was said or done by whoever wrote it; in {user}'s message \"I\" means {user}):\n\
{user} (the player) wrote:\n{}\n\n{reply} replied:\n{}",
            self.user_text, self.reply_text,
        )
    }
}

/// "- fact" / "* fact" / "fact" lines, without blanks.
fn fact_lines(response: &str) -> impl Iterator<Item = &str> {
    response.lines().map(|line| line.trim().trim_start_matches(['-', '*', ' '])).filter(|line| !line.is_empty())
}

impl Extractor<'_> {
    async fn ask(&self, prompt: String) -> Result<String> {
        let provider = self.providers.get(self.provider_id).map_err(|e| Error::Other(e.message))?;
        let request = LlmRequest {
            model: self.model.to_string(),
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, prompt)],
            generation_config: Default::default(),
            enable_web_search: None,
        };
        let response = provider.get_completion(request).await.map_err(|e| Error::Other(e.message))?;
        Ok(response.delta_text)
    }

    /// Assistant chats: new durable facts about the user, kept for future Assistant chats.
    pub async fn memories(&self, exchange: &Exchange<'_>) -> Result<()> {
        if exchange.user_text.trim().is_empty() {
            return Ok(());
        }
        let existing = self.db.with(user_memories::get_all)?;
        let existing_list = if existing.is_empty() {
            "(none yet)".to_string()
        } else {
            existing.iter().map(|m| format!("- {}", m.content)).collect::<Vec<_>>().join("\n")
        };

        let prompt = format!(
            "You are extracting durable facts about the USER from one exchange in an ongoing assistant conversation, for long-term memory recall in future chats.\n\n\
Facts already remembered (do not repeat these):\n{existing_list}\n\n\
Exchange:\nUser: {user}\nAssistant: {reply}\n\n\
List any NEW, durable, specific facts about the user worth remembering long-term (identity, preferences, ongoing projects, constraints) - one per line, no bullets, no commentary, no quotes. Skip trivia, small talk, or anything already remembered. If there is nothing new and durable, output exactly: NONE",
            user = exchange.user_text,
            reply = exchange.reply_text,
        );
        let response = self.ask(prompt).await?;

        self.db.with(|c| {
            for fact in fact_lines(&response).filter(|f| !f.eq_ignore_ascii_case("NONE")) {
                user_memories::add(c, &UserMemory::new(fact))?;
            }
            Ok(())
        })
    }

    /// Roleplays: adds, updates, completes or fails quests based on the exchange.
    pub async fn quests(&self, conversation_id: Uuid, exchange: &Exchange<'_>) -> Result<()> {
        if exchange.user_text.trim().is_empty() {
            return Ok(());
        }
        let existing = self.db.with(|c| quest_repo::get_by_conversation(c, conversation_id))?;
        let existing_list = if existing.is_empty() {
            "(none yet)".to_string()
        } else {
            existing
                .iter()
                .map(|q| format!("- {} ({}): {}", q.title, status_name(q.status), q.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let prompt = format!(
            "You are maintaining a quest/objective log for an ongoing roleplay, based on one exchange.\n\n\
Current quests:\n{existing_list}\n\n\
{transcript}\n\n\
Output zero or more lines, one per change, in exactly this grammar:\n\
ADD: <title> :: <one-line description>\n\
UPDATE: <existing title, exact> :: <new one-line description>\n\
COMPLETE: <existing title, exact>\n\
FAIL: <existing title, exact>\n\
Only ADD a new quest for a concrete objective introduced in this exchange. Only UPDATE/COMPLETE/FAIL a quest whose title exactly matches one already listed above. If nothing changed, output exactly: NONE",
            transcript = exchange.roleplay_transcript(),
        );
        let response = self.ask(prompt).await?;

        self.db.with(|c| {
            for action in quest_log::parse(&response, &existing) {
                use quest_log::QuestLogActionType::*;
                match action.action_type {
                    Add => {
                        quest_repo::add(
                            c,
                            &QuestLogEntry::new(conversation_id, action.title, action.description.unwrap_or_default()),
                        )?;
                    }
                    Update | Complete | Fail => {
                        let Some(mut quest) = existing.iter().find(|q| Some(q.id) == action.existing_quest_id).cloned() else {
                            continue;
                        };
                        match action.action_type {
                            Update => {
                                if let Some(description) = action.description {
                                    quest.description = description;
                                }
                            }
                            Complete => quest.status = QuestStatus::Completed,
                            _ => quest.status = QuestStatus::Failed,
                        }
                        quest.updated_at = chrono::Utc::now();
                        quest_repo::update(c, &quest)?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Roleplays: what each present character personally learned, as private notebook entries; a
    /// notebook that has grown past [`CONSOLIDATE_ABOVE_COUNT`] is then condensed.
    pub async fn notebooks(&self, conversation_id: Uuid, characters: &[Character], exchange: &Exchange<'_>) -> Result<()> {
        if exchange.user_text.trim().is_empty() || characters.is_empty() {
            return Ok(());
        }
        let existing = self.db.with(|c| notebook_repo::get_by_conversation(c, conversation_id))?;
        let existing_block = characters
            .iter()
            .map(|character| {
                let own: Vec<_> = existing.iter().filter(|e| e.character_id == character.id).collect();
                let list = if own.is_empty() {
                    "(none yet)".to_string()
                } else {
                    own.iter().map(|e| format!("- {}", e.content)).collect::<Vec<_>>().join("\n")
                };
                format!("{}:\n{list}", character.name)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let names = characters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");

        let prompt = format!(
            "You are extracting what specific characters in an ongoing roleplay have PERSONALLY learned about the player or events from one exchange, for their own private long-term memory. A character's notebook is private to them - other characters do not automatically know what's in it.\n\n\
Characters present: {names}\n\n\
What each character already knows (do not repeat these):\n{existing_block}\n\n\
{transcript}\n\n\
For each character who personally learned something NEW and worth remembering (a fact about the player, a secret, an event they witnessed), output one line as \"<CharacterName>: <fact>\". Name who did what - the player is {player}, never one of the characters. Skip characters who learned nothing new or weren't present. If nobody learned anything new, output exactly: NONE",
            transcript = exchange.roleplay_transcript(),
            player = exchange.user_name,
        );
        let response = self.ask(prompt).await?;

        let mut counts: HashMap<Uuid, usize> =
            characters.iter().map(|c| (c.id, existing.iter().filter(|e| e.character_id == c.id).count())).collect();
        self.db.with(|c| {
            for addition in notebook::parse(&response, characters) {
                notebook_repo::add(c, &CharacterNotebookEntry::new(conversation_id, addition.character_id, addition.content))?;
                *counts.entry(addition.character_id).or_default() += 1;
            }
            Ok(())
        })?;

        for character in characters.iter().filter(|c| counts.get(&c.id).copied().unwrap_or(0) > CONSOLIDATE_ABOVE_COUNT) {
            self.consolidate(conversation_id, character).await?;
        }
        Ok(())
    }

    /// Re-reads one character's entries (they may have changed since the snapshot above) and, if
    /// still over the threshold, replaces them with the model's condensed version. A response with
    /// no usable line leaves the notebook alone - a failed consolidation must never wipe it out.
    async fn consolidate(&self, conversation_id: Uuid, character: &Character) -> Result<()> {
        let current: Vec<_> = self
            .db
            .with(|c| notebook_repo::get_by_conversation(c, conversation_id))?
            .into_iter()
            .filter(|e| e.character_id == character.id)
            .collect();
        if current.len() <= CONSOLIDATE_ABOVE_COUNT {
            return Ok(());
        }

        let list = current.iter().map(|e| format!("- {}", e.content)).collect::<Vec<_>>().join("\n");
        let prompt = format!(
            "The following is {name}'s private notebook from an ongoing roleplay - things they've personally learned about the player and events over time. It has grown long. Condense it into as few entries as needed to preserve every distinct fact, merging duplicates and closely related notes, dropping anything truly redundant, and keeping the wording concise. Do not invent or drop any real information.\n\n\
Current notebook:\n{list}\n\n\
Output the condensed notebook as one fact per line, no bullets, no numbering, no commentary.",
            name = character.name,
        );
        let response = self.ask(prompt).await?;

        let condensed: Vec<&str> = fact_lines(&response).collect();
        if condensed.is_empty() {
            return Ok(());
        }
        self.db.with(|c| {
            notebook_repo::delete_all_for_character(c, conversation_id, character.id)?;
            for content in condensed {
                notebook_repo::add(c, &CharacterNotebookEntry::new(conversation_id, character.id, content))?;
            }
            Ok(())
        })
    }
}

/// The status as the C# app printed it into the prompt (`QuestStatus.ToString()`).
fn status_name(status: QuestStatus) -> &'static str {
    match status {
        QuestStatus::Active => "Active",
        QuestStatus::Completed => "Completed",
        QuestStatus::Failed => "Failed",
    }
}
