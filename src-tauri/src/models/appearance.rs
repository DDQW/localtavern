use serde::{Deserialize, Serialize};

/// Picks the quick appearance builder offers. Only the section matching `sex` is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sex {
    Male,
    Female,
}

/// The structured picks behind a character's or persona's appearance text, kept so the builder
/// reopens with them. Every trait is optional free text: the UI offers preset values plus a
/// custom entry, and missing keys deserialize to empty so older rows keep loading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub sex: Option<Sex>,
    pub height_cm: Option<u16>,
    pub weight_kg: Option<u16>,
    pub build: Option<String>,
    pub muscle_tone: Option<String>,
    pub skin_tone: Option<String>,
    pub hair_color: Option<String>,
    pub hair_length: Option<String>,
    pub hair_style: Option<String>,
    pub eye_color: Option<String>,
    pub body_hair: Option<String>,
    /// Freckles, tattoos, scars, piercings, glasses, ...
    pub marks: Vec<String>,
    pub female: FemaleTraits,
    pub male: MaleTraits,
    /// Only rendered when something in it is filled in.
    pub adult: AdultTraits,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FemaleTraits {
    pub cup_size: Option<String>,
    pub bust: Option<String>,
    pub waist: Option<String>,
    pub hips: Option<String>,
    pub butt: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MaleTraits {
    pub shoulders: Option<String>,
    pub chest: Option<String>,
    pub facial_hair: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdultTraits {
    pub nipples: Option<String>,
    pub pubic_hair: Option<String>,
    pub genitals: Option<String>,
    /// Male only.
    pub circumcision: Option<String>,
    pub notes: Option<String>,
}
