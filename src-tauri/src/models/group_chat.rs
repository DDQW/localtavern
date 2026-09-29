use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupActivationMode {
    Manual,
    RoundRobin,
    TriggerWord,
}
int_enum!(GroupActivationMode { Manual = 0, RoundRobin = 1, TriggerWord = 2 });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChat {
    pub id: Uuid,
    pub name: String,
    pub character_ids: Vec<Uuid>,
    pub activation_mode: GroupActivationMode,
    pub allow_self_responses: bool,
}

impl Default for GroupChat {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            character_ids: Vec::new(),
            activation_mode: GroupActivationMode::Manual,
            allow_self_responses: false,
        }
    }
}
