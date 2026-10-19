//! Best-effort background enrichment after a reply: pulling durable facts about the user out of
//! Assistant chats. It asks the model a single question about one exchange and applies the answer.
//! It runs detached from the reply (see `ChatEngine::spawn_enrichment`), and a failure here never
//! reaches the chat - the visible reply already succeeded.

use crate::db::Db;
use crate::error::{Error, Result};
use crate::llm::{LlmMessage, LlmRequest, LlmRole};
use crate::models::UserMemory;
use crate::providers::LlmProviderRegistry;
use crate::repo::user_memories;

/// Where to send the question and what to store the answer in.
pub struct Extractor<'a> {
    pub db: &'a Db,
    pub providers: &'a LlmProviderRegistry,
    pub provider_id: &'a str,
    pub model: &'a str,
}

/// One exchange: the user's message and the reply to it.
pub struct Exchange<'a> {
    pub user_text: &'a str,
    pub reply_text: &'a str,
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
}
