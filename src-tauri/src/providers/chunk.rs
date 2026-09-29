use crate::models::LlmUsage;

/// Raw bytes of an image the model returned inline (e.g. an image-generation model's output) - not
/// yet saved to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmGeneratedImage {
    pub data: Vec<u8>,
    pub mime_type: String,
}

/// One incremental piece of a model reply.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LlmResponseChunk {
    pub delta_text: String,
    pub is_final: bool,
    pub usage: Option<LlmUsage>,
    pub finish_reason: Option<String>,
    /// The model's reasoning/"thinking" text for this delta, kept apart from `delta_text` so callers
    /// can hide it by default.
    pub thought_delta_text: String,
    /// Any images this chunk carried inline - only populated by image-generation-capable models.
    pub images: Option<Vec<LlmGeneratedImage>>,
}

impl LlmResponseChunk {
    pub fn text(delta: impl Into<String>) -> Self {
        Self { delta_text: delta.into(), ..Default::default() }
    }
}
