use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub id: Uuid,
    pub name: String,
    pub avatar_path: Option<String>,
    pub description: String,
    pub is_default: bool,
}

impl Default for Persona {
    fn default() -> Self {
        Self { id: Uuid::new_v4(), name: String::new(), avatar_path: None, description: String::new(), is_default: false }
    }
}
