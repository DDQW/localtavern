//! Pictures the user sends are described once, in the background after the reply to them. From the
//! user's next message on, the assembler sends that description instead of the picture: resending
//! the image made the model decode it again on every reply (and cost its tokens every time).

use std::path::Path;

use uuid::Uuid;

use super::{report, ChatEngine};
use crate::llm::{LlmAttachment, LlmGenerationConfig, LlmMessage, LlmRequest, LlmRole};
use crate::models::{ChatMessage, MessageAttachment, MessageRole};
use crate::repo::conversations;

/// The request for one picture's description. The message it came with says what the user wanted
/// from it, so the description keeps what matters for that.
fn describe_prompt(user_text: &str) -> String {
    let mut prompt = String::from(
        "Describe this picture so a conversation about it can go on without seeing it again. Cover \
who or what is in it, people's looks, expressions, poses and clothing, the setting, colours and \
lighting, and the mood. Copy any text in it word for word; for a screenshot or document, give its \
content. Plain prose, no preamble, no opinions.",
    );
    if !user_text.trim().is_empty() {
        prompt.push_str(&format!("\n\nIt was sent with this message: {}", user_text.trim()));
    }
    prompt
}

impl ChatEngine {
    /// Describes the pictures in the conversation's user messages that don't have a description
    /// yet, one at a time with the chat's own model. Best-effort: a failed or empty answer leaves
    /// the picture to be sent as is, and the next finished reply tries again.
    pub(super) fn spawn_picture_descriptions(&self, conversation_id: Uuid, provider_id: &str, model: &str) {
        let Ok(messages) = self.db().with(|c| conversations::get_messages(c, conversation_id)) else { return };
        let pending: Vec<ChatMessage> = {
            let mut running = self.inner.describing.lock().unwrap();
            messages
                .into_iter()
                .filter(|m| m.role == MessageRole::User && m.attachments.iter().any(|a| a.description.is_none()))
                .filter(|m| running.insert(m.id))
                .collect()
        };
        if pending.is_empty() {
            return;
        }
        let engine = self.clone();
        let (provider_id, model) = (provider_id.to_string(), model.to_string());
        tokio::spawn(async move {
            for message in pending {
                // A picture whose file has gone missing would be described from the text alone.
                let undescribed =
                    message.attachments.iter().filter(|a| a.description.is_none() && Path::new(&a.image_path).is_file());
                for attachment in undescribed {
                    if let Some(description) = engine.describe_picture(&provider_id, &model, &message.text, attachment).await {
                        engine.save_description(message.id, &attachment.image_path, description);
                    }
                }
                engine.inner.describing.lock().unwrap().remove(&message.id);
            }
        });
    }

    async fn describe_picture(
        &self,
        provider_id: &str,
        model: &str,
        user_text: &str,
        attachment: &MessageAttachment,
    ) -> Option<String> {
        let mut message = LlmMessage::new(LlmRole::User, describe_prompt(user_text));
        message.attachments =
            Some(vec![LlmAttachment { image_path: attachment.image_path.clone(), mime_type: attachment.mime_type.clone() }]);
        let request = LlmRequest {
            model: model.to_string(),
            system_instruction: None,
            history: vec![message],
            generation_config: LlmGenerationConfig::without_reasoning(),
            enable_web_search: None,
        };
        let provider = self.inner.providers.get(provider_id).ok()?;
        let response = provider.get_completion(request).await.ok()?;
        let description = response.delta_text.trim();
        (!description.is_empty()).then(|| description.to_string())
    }

    /// Stores the description on the message as it is now, so an edit made meanwhile survives; a
    /// message deleted meanwhile is left gone.
    fn save_description(&self, message_id: Uuid, image_path: &str, description: String) {
        let saved = self.db().with(|c| {
            let Some(mut message) = conversations::get_message(c, message_id)? else { return Ok(()) };
            let Some(attachment) = message.attachments.iter_mut().find(|a| a.image_path == image_path) else { return Ok(()) };
            attachment.description = Some(description);
            conversations::set_attachments(c, message_id, &message.attachments)
        });
        report("picture description", saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_carries_the_message_the_picture_came_with() {
        assert!(!describe_prompt("  ").contains("sent with this message"));
        assert!(describe_prompt("what breed is this?").ends_with("It was sent with this message: what breed is this?"));
    }
}
