use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorldInfoPosition {
    BeforeCharacterDescription,
    AfterCharacterDescription,
    AtDepth,
}
int_enum!(WorldInfoPosition { BeforeCharacterDescription = 0, AfterCharacterDescription = 1, AtDepth = 2 });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInfoEntry {
    pub id: Uuid,
    pub keys: Vec<String>,
    pub secondary_keys: Option<Vec<String>>,
    pub content: String,
    pub case_sensitive: bool,
    pub enabled: bool,
    pub insertion_order: i32,
    pub position: WorldInfoPosition,
    pub depth: i32,
}

impl Default for WorldInfoEntry {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            keys: Vec::new(),
            secondary_keys: None,
            content: String::new(),
            case_sensitive: false,
            enabled: true,
            insertion_order: 0,
            position: WorldInfoPosition::BeforeCharacterDescription,
            depth: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lorebook {
    pub id: Uuid,
    pub name: String,
    pub entries: Vec<WorldInfoEntry>,
    pub scan_depth: i32,
    pub token_budget: i32,
}

impl Default for Lorebook {
    fn default() -> Self {
        Self { id: Uuid::new_v4(), name: String::new(), entries: Vec::new(), scan_depth: 4, token_budget: 1000 }
    }
}
