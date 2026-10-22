//! The chat side of picture generation: the conversation's own model writes the description (it
//! knows the story, the cards and the lore), and the finished picture lands in the chat as an
//! [`MessageRole::Image`] message. The rendering itself is [`crate::imagegen`]'s job.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{images, load_reply_context, user_message, AssembleOptions, ChatEngine};
use crate::error::{Error, Result};
use crate::imagegen::prompts::{self, ImageMode};
use crate::imagegen::{GeneratedImage, ImageGenSettings, PromptStyle};
use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRequest, LlmRole};
use crate::models::{Character, ChatMessage, MessageAttachment, MessageRole, PromptBlockType, ReasoningEffort};
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
    /// For a photo sent or taken in the story: what the reply said it showed. Such a picture is part
    /// of the story, so the model is told about it (see [`super::session::chat_history`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    /// For a video sent in the story: what the reply said it showed. Rendered with the ComfyUI video
    /// workflow instead of the still one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
}

/// A picture message for `draft`, not saved yet and with no picture in it.
fn picture_message(conversation_id: Uuid, draft: &ImagePromptDraft) -> Result<ChatMessage> {
    let mut message = ChatMessage::new(conversation_id, MessageRole::Image, draft.prompt.trim());
    message.speaker_character_id = draft.character_id;
    message.raw_prompt_text = Some(serde_json::to_string(draft)?);
    Ok(message)
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
        // A tool offer or a web search would only get in the way, and a tag list needs no long
        // reasoning (a thinking model at a high level could otherwise think for minutes).
        request.enable_web_search = Some(false);
        request.generation_config.reasoning_effort = Some(ReasoningEffort::Low);

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

        // Permanent visual identity keeps pictures looking like them (base look, else appearance,
        // else written description). Also used for scenes of a one-on-one chat.
        let looked = match (mode, drawn) {
            (_, Some(character)) => Some(character),
            (ImageMode::Scene | ImageMode::LastMessage, None) if ctx.characters.len() == 1 => ctx.characters.first(),
            _ => None,
        };
        let (character_prefix, locked) =
            looked.map(|c| prompts::character_visual_identity(c)).unwrap_or_default();
        let char_positive = looked.map(|c| c.picture_style.prompt_prefix.as_str()).unwrap_or("");
        let char_negative = looked.map(|c| c.picture_style.negative_prompt.as_str()).unwrap_or("");
        let prefix = prompts::merge_prompt_parts(&[&settings.prompt_prefix, char_positive]);
        // When base-look tags lock identity, drop hair/skin/race overrides the model put in the shot
        // and put the look last so the image server weights it.
        let prompt = if locked && !character_prefix.is_empty() {
            prompts::combine(&prefix, &prompts::scrub_appearance_overrides(&description), &character_prefix)
        } else {
            prompts::combine(&prefix, &character_prefix, &description)
        };

        let (width, height) = prompts::size_for(mode, settings.width, settings.height);
        Ok(Some(ImagePromptDraft {
            mode,
            prompt,
            negative_prompt: prompts::merge_prompt_parts(&[&settings.negative_prompt, char_negative]),
            width,
            height,
            character_id: drawn.map(|c| c.id),
            photo: None,
            video: None,
        }))
    }

    /// Stores a rendered picture and adds it to the chat. The prompt is the message text, and the
    /// whole draft (negative prompt and size too) is kept as its raw prompt, so "draw again" can
    /// repeat it exactly.
    pub fn add_picture(&self, conversation_id: Uuid, draft: &ImagePromptDraft, image: &GeneratedImage) -> Result<ChatMessage> {
        let mut message = picture_message(conversation_id, draft)?;
        message.attachments = vec![self.store_picture(image)?];
        self.db().with(|c| {
            conversations::add_message(c, &message)?;
            Ok(())
        })?;
        Ok(message)
    }

    /// Keeps a story photo whose render failed in the chat, as a picture message with no picture, so
    /// it can be drawn again in its place and the model still knows it was sent.
    pub fn add_failed_photo(&self, conversation_id: Uuid, draft: &ImagePromptDraft) -> Result<ChatMessage> {
        if draft.photo.is_none() && draft.video.is_none() {
            return Err(Error::Other("Only a photo or video from the story is kept when it can't be drawn.".into()));
        }
        let message = picture_message(conversation_id, draft)?;
        self.db().with(|c| {
            conversations::add_message(c, &message)?;
            Ok(())
        })?;
        Ok(message)
    }

    /// Puts a rendered picture into an existing picture message (one whose render had failed).
    pub fn fill_picture(&self, message_id: Uuid, image: &GeneratedImage) -> Result<ChatMessage> {
        let mut message = self
            .db()
            .with(|c| conversations::get_message(c, message_id))?
            .filter(|m| m.role == MessageRole::Image)
            .ok_or_else(|| Error::Other("That picture is gone.".into()))?;
        let old = std::mem::replace(&mut message.attachments, vec![self.store_picture(image)?]);
        self.db().with(|c| conversations::set_attachments(c, message_id, &message.attachments))?;
        for attachment in &old {
            images::delete_if_owned(&self.inner.images_dir, std::path::Path::new(&attachment.image_path));
        }
        Ok(message)
    }

    fn store_picture(&self, image: &GeneratedImage) -> Result<MessageAttachment> {
        let path = images::store_bytes(&self.inner.images_dir, &image.data, &image.mime_type)?;
        Ok(MessageAttachment { image_path: path.to_string_lossy().into_owned(), mime_type: image.mime_type.clone(), description: None })
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

    /// Has the last-selected model turn a character's description into the two prompt lines every
    /// picture of them shares: permanent features ("base look") and their usual clothing.
    pub async fn describe_look(&self, character: &Character, style: PromptStyle) -> Result<(String, String)> {
        let description = character.description.trim();
        if description.is_empty() {
            return Err(Error::Other("Write a description first - the look is taken from it.".into()));
        }
        let (provider_id, model) = self.db().with(|c| self.default_provider_and_model(c))?;
        if model.trim().is_empty() {
            return Err(Error::Other("No model selected - pick one, or refresh the model list in Settings.".into()));
        }
        let format = match style {
            PromptStyle::Tags => "comma-separated image-generator tags (e.g. \"1girl, long silver hair, blue eyes, freckles\")",
            PromptStyle::Natural => "one short plain sentence each",
        };
        let name = if character.name.trim().is_empty() { "the character" } else { character.name.trim() };
        let prompt = format!(
            "From this description of {name}, write two lines for an image generator, as {format}.
LOOK: only permanent physical features - species or race, gender, apparent age, build, skin, face, hair (colour, length, style), eyes, and distinctive marks. No clothing, no name, no personality.
OUTFIT: what they usually wear, if the description says; otherwise a fitting everyday outfit.
Reply with exactly those two lines, starting with LOOK: and OUTFIT:.

Description:
{description}"
        );
        let request = LlmRequest {
            model,
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, prompt)],
            generation_config: LlmGenerationConfig { reasoning_effort: Some(ReasoningEffort::Low), ..Default::default() },
            enable_web_search: None,
        };
        let provider = self.inner.providers.get(&provider_id).map_err(|e| Error::Other(user_message(&e)))?;
        let response = provider.get_completion(request).await.map_err(|e| Error::Other(user_message(&e)))?;
        let (look, outfit) = prompts::parse_look(&response.delta_text);
        if look.is_empty() {
            return Err(Error::Other("The model returned an empty description - try again.".into()));
        }
        Ok((look, outfit))
    }
}
