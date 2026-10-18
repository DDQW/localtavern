//! The chat side of picture generation: the conversation's own model writes the description (it
//! knows the story, the cards and the lore), and the finished picture lands in the chat as an
//! [`MessageRole::Image`] message. The rendering itself is [`crate::imagegen`]'s job.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{images, load_reply_context, user_message, AssembleOptions, ChatEngine};
use crate::error::{Error, Result};
use crate::imagegen::prompts::{self, ImageMode};
use crate::imagegen::{self, GeneratedImage, ImageGenSettings};
use crate::llm::{LlmMessage, LlmRole};
use crate::models::{ChatMessage, MessageAttachment, MessageRole, PromptBlockType};
use crate::prompting::roles;
use crate::repo::conversations;

/// A picture prompt ready for review: the user can edit any of it before it's rendered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePromptDraft {
    pub mode: ImageMode,
    /// The full prompt the server will get, prefixes included.
    pub prompt: String,
    pub negative_prompt: String,
    pub width: u32,
    pub height: u32,
    /// Who the picture is of (character modes), shown as the picture's speaker.
    pub character_id: Option<Uuid>,
}

/// The assembler adds its own trailing notes (the world clock, an author's note), which would put
/// the picture request second-to-last. Models follow the very last instruction best, so it's moved
/// onto the end of the final user turn.
fn keep_instruction_last(history: &mut Vec<LlmMessage>, instruction: &str) {
    let Some(at) = history.iter().rposition(|m| m.text.contains(instruction)) else {
        return;
    };
    if at + 1 == history.len() || history.last().is_none_or(|m| m.role != LlmRole::User) {
        return;
    }
    let text = history[at].text.replace(instruction, "");
    let text = text.trim_end();
    if text.is_empty() && history[at].attachments.is_none() {
        history.remove(at);
    } else {
        history[at].text = text.to_string();
    }
    let last = history.last_mut().expect("checked above");
    last.text = format!("{}\n\n{instruction}", last.text.trim_end());
}

impl ChatEngine {
    /// Asks the conversation's model to describe the picture for `mode`. `character_id` picks the
    /// subject in a group; otherwise it's the last character who spoke. `None` when stopped.
    pub async fn image_prompt(
        &self,
        conversation_id: Uuid,
        mode: ImageMode,
        character_id: Option<Uuid>,
        settings: &ImageGenSettings,
    ) -> Result<Option<ImagePromptDraft>> {
        let (token, _guard) = self.claim(conversation_id)?;
        let ctx = self.db().with(|c| load_reply_context(c, conversation_id))?;
        if ctx.conversation.model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }

        let last_speaker = ctx
            .messages
            .iter()
            .rev()
            .filter(|m| m.role == MessageRole::Character)
            .find_map(|m| m.speaker_character_id)
            .and_then(|id| ctx.characters.iter().find(|c| c.id == id));
        let chosen = character_id.and_then(|id| ctx.characters.iter().find(|c| c.id == id));
        let subject = chosen.or(last_speaker).or(ctx.characters.first());

        let user_name = roles::user_name(ctx.persona.as_ref());
        let (subject_name, drawn) = match mode {
            ImageMode::Character | ImageMode::Face => {
                let character = subject.ok_or_else(|| Error::Other("This chat has no character to draw.".into()))?;
                (character.name.as_str(), Some(character))
            }
            ImageMode::Persona => (user_name, None),
            ImageMode::Scene | ImageMode::LastMessage => ("", None),
        };
        if mode == ImageMode::LastMessage && ctx.messages.is_empty() {
            return Err(Error::Other("There is no message to draw yet.".into()));
        }

        let instruction = prompts::instruction(mode, settings.prompt_style, subject_name);
        let mut history = ctx.messages.clone();
        match history.last_mut() {
            // Gemini rejects two user turns in a row, so the request rides on the user's message.
            Some(last) if last.role == MessageRole::User => {
                let mut folded = ChatMessage::new(conversation_id, MessageRole::User, format!("{}\n\n{instruction}", last.text));
                folded.speaker_character_id = last.speaker_character_id;
                *last = folded;
            }
            _ => history.push(ChatMessage::new(conversation_id, MessageRole::User, instruction.clone())),
        }

        // The preset's "write the next reply" prompt would fight the request.
        let mut preset = ctx.preset.clone();
        preset.blocks.retain(|b| b.block_type != PromptBlockType::SystemPrompt);
        let mut request = self.assemble(
            &ctx,
            &history,
            AssembleOptions { preset: Some(&preset), speaker: drawn.or(subject), as_user: true, ..Default::default() },
        )?;
        keep_instruction_last(&mut request.history, &instruction);
        request.system_instruction = Some(match request.system_instruction.take() {
            Some(system) if !system.trim().is_empty() => format!(
                "{system}\n\nWhen asked for a picture description, step out of the story and give only the description."
            ),
            _ => "When asked for a picture description, give only the description.".to_string(),
        });
        // A tool offer or a web search would only get in the way.
        request.enable_web_search = Some(false);

        let provider = self.inner.providers.get(&ctx.conversation.provider_id).map_err(|e| Error::Other(user_message(&e)))?;
        let outcome = tokio::select! {
            biased;
            _ = token.cancelled() => return Ok(None),
            outcome = provider.get_completion(request) => outcome,
        };
        let response = outcome.map_err(|e| Error::Other(user_message(&e)))?;
        let description = prompts::clean_model_prompt(&response.delta_text);
        if description.is_empty() {
            return Err(Error::Other("The model returned an empty description - try again.".into()));
        }

        // A character's own prefix (a LoRA, a fixed look) applies to their pictures, and to scenes
        // in a one-on-one chat where they're surely in frame.
        let prefixed = match (mode, drawn) {
            (_, Some(character)) => Some(character.id),
            (ImageMode::Scene | ImageMode::LastMessage, None) if ctx.characters.len() == 1 => Some(ctx.characters[0].id),
            _ => None,
        };
        let character_prefix = match prefixed {
            Some(id) => self.db().with(|c| imagegen::settings::character_prefix(c, id))?,
            None => String::new(),
        };

        let (width, height) = prompts::size_for(mode, settings.width, settings.height);
        Ok(Some(ImagePromptDraft {
            mode,
            prompt: prompts::combine(&settings.prompt_prefix, &character_prefix, &description),
            negative_prompt: settings.negative_prompt.clone(),
            width,
            height,
            character_id: drawn.map(|c| c.id),
        }))
    }

    /// Stores a rendered picture and adds it to the chat. The prompt is the message text, and the
    /// whole draft (negative prompt and size too) is kept as its raw prompt, so "draw again" can
    /// repeat it exactly.
    pub fn add_picture(&self, conversation_id: Uuid, draft: &ImagePromptDraft, image: &GeneratedImage) -> Result<ChatMessage> {
        let path = images::store_bytes(&self.inner.images_dir, &image.data, &image.mime_type)?;
        let mut message = ChatMessage::new(conversation_id, MessageRole::Image, draft.prompt.trim());
        message.speaker_character_id = draft.character_id;
        message.raw_prompt_text = Some(serde_json::to_string(draft)?);
        message.attachments = vec![MessageAttachment {
            image_path: path.to_string_lossy().into_owned(),
            mime_type: image.mime_type.clone(),
        }];
        self.db().with(|c| {
            conversations::add_message(c, &message)?;
            Ok(())
        })?;
        Ok(message)
    }

    /// Deletes a picture message and, when it lives in the app's own folder, its file.
    pub fn delete_picture(&self, message_id: Uuid) -> Result<()> {
        let message = self
            .db()
            .with(|c| conversations::get_message(c, message_id))?
            .ok_or_else(|| Error::Other("Message not found.".into()))?;
        if message.role != MessageRole::Image {
            return Err(Error::Other("That message isn't a picture.".into()));
        }
        self.db().with(|c| conversations::delete_message(c, message_id))?;
        for attachment in &message.attachments {
            images::delete_if_owned(&self.inner.images_dir, std::path::Path::new(&attachment.image_path));
        }
        Ok(())
    }
}
