use chrono::NaiveDateTime;
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
    /// Each reply may hand the user's message on to a member who hasn't answered it yet (the model
    /// decides, with a `[next: ...]` tag), so several can answer one message.
    pub joint_replies: bool,
    /// A member may take the user aside into a private talk on their own (`[private]` tag).
    pub private_talks: bool,
    /// The group's own scenario. When set, it stands in for the members' own scenarios (each card's
    /// is written for a chat with that character alone).
    #[serde(default)]
    pub scenario: String,
    /// Where and when the group's new chats are set; unset parts fall back to the members'.
    #[serde(default)]
    pub setting: GroupSetting,
}

/// Where and when a group's story is set, for every chat started with the group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GroupSetting {
    /// ISO 3166-1 alpha-2, e.g. "DE"; picks the public holidays.
    pub country_code: Option<String>,
    /// Subdivision for regional holidays, e.g. "DE-BY".
    pub region_code: Option<String>,
    /// When a new chat's story clock starts.
    pub story_start: Option<NaiveDateTime>,
}

impl Default for GroupChat {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            character_ids: Vec::new(),
            activation_mode: GroupActivationMode::Manual,
            allow_self_responses: false,
            joint_replies: false,
            private_talks: true,
            scenario: String::new(),
            setting: GroupSetting::default(),
        }
    }
}
