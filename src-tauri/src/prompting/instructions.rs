//! Every text the app adds to a chat request on its own - roleplay rules, tags, the story clock,
//! photos, group and private-talk notes, Continue and Impersonate - as templates the user can
//! reword in Settings, plus the switches and word list around them. A text left empty uses the
//! built-in one. Placeholders are words in braces (`{char}`, `{user}`...); each text lists its own.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::repo::app_settings;

const SETTINGS_KEY: &str = "prompts.instructions";

// Roleplay rules
pub const SYSTEM_NOTE: &str = "systemNote";
pub const RULES_SOLO: &str = "rulesSolo";
pub const RULES_GROUP: &str = "rulesGroup";
pub const RULES_COMMON: &str = "rulesCommon";
pub const RULES_GROUP_MEMBERS: &str = "rulesGroupMembers";
pub const RULES_USER_ONLY: &str = "rulesUserOnly";
// Tags
pub const EMOTION_TAG: &str = "emotionTag";
pub const TIME_SKIP_TAG: &str = "timeSkipTag";
// Photos
pub const PHOTO: &str = "photo";
pub const PHOTO_INTIMATE: &str = "photoIntimate";
pub const USER_PHOTO: &str = "userPhoto";
// Pictures the user sends
pub const PICTURE_SHOWN: &str = "pictureShown";
pub const PICTURE_SHOWN_ASSISTANT: &str = "pictureShownAssistant";
pub const PICTURE_NOTE_ONE: &str = "pictureNoteOne";
pub const PICTURE_NOTE_MANY: &str = "pictureNoteMany";
// Group chats
pub const NEXT_SPEAKER: &str = "nextSpeaker";
pub const PRIVATE: &str = "private";
pub const GROUP_TURN: &str = "groupTurn";
// Private talks
pub const PRIVATE_RULES: &str = "privateRules";
pub const PRIVATE_MEMORY: &str = "privateMemory";
pub const GROUP_NOTE: &str = "groupNote";
pub const GROUP_EVENT: &str = "groupEvent";
pub const STEPPED_AWAY: &str = "steppedAway";
pub const GROUP_SUMMARY: &str = "groupSummary";
// Story clock and world
pub const WORLD_NOW: &str = "worldNow";
pub const WORLD_PLACE: &str = "worldPlace";
pub const WORLD_HOLIDAY: &str = "worldHoliday";
pub const WORLD_OBSERVANCE: &str = "worldObservance";
pub const WORLD_WEEKEND: &str = "worldWeekend";
pub const WORLD_WEEKDAY: &str = "worldWeekday";
pub const WORLD_UPCOMING: &str = "worldUpcoming";
pub const WORLD_SCHOOL_OUT: &str = "worldSchoolOut";
pub const WORLD_SCHOOL_SOON: &str = "worldSchoolSoon";
pub const WORLD_ROUTINES: &str = "worldRoutines";
pub const WORLD_METERS: &str = "worldMeters";
pub const WORLD_BIRTHDAYS: &str = "worldBirthdays";
// Cards, persona and history
pub const PERSONA_LABEL: &str = "personaLabel";
pub const PERSONALITY_LABEL: &str = "personalityLabel";
pub const CAST_LABEL: &str = "castLabel";
pub const MEMORIES_HEADER: &str = "memoriesHeader";
pub const SUMMARY_STORY: &str = "summaryStory";
pub const SUMMARY_ASSISTANT: &str = "summaryAssistant";
// Continue, Impersonate, Assistant
pub const CONTINUE_NUDGE: &str = "continueNudge";
pub const IMPERSONATE: &str = "impersonate";
pub const POWERSHELL_TOOL: &str = "powershellTool";

/// One editable instruction, as Settings shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instruction {
    pub key: &'static str,
    /// Where Settings lists it.
    pub group: &'static str,
    pub title: &'static str,
    /// When the app adds it.
    pub help: &'static str,
    pub placeholders: &'static [&'static str],
    pub default: &'static str,
}

const ROLES: &str = "Roleplay rules";
const TAGS: &str = "Tags at the end of replies";
const PHOTOS: &str = "Photos";
const PICTURES: &str = "Pictures you send";
const GROUPS: &str = "Group chats";
const TALKS: &str = "Private talks";
const WORLD: &str = "Story clock and world";
const CARDS: &str = "Cards, persona and history";
const OTHER: &str = "Continue, Impersonate and Assistant";

pub const ALL: &[Instruction] = &[
    // ---- roleplay rules
    Instruction {
        key: SYSTEM_NOTE,
        group: ROLES,
        title: "System note wrapper",
        help: "Wraps everything the app slips into the history as a user-role turn (author's note, world state, lore at depth, the notes below), so it isn't read as your character speaking. The rules below tell the model about this form - change both together.",
        placeholders: &["{text}"],
        default: "[System note: {text}]",
    },
    Instruction {
        key: RULES_SOLO,
        group: ROLES,
        title: "Who is who (one character)",
        help: "First part of the rules sent with every roleplay reply with one character.",
        placeholders: &["{user}", "{char}"],
        default: "Roles: the user plays {user}; you play {char}. Every user-role message is written by {user}: everything in it, \
*actions* included, is what {user} says, does, thinks or feels - never {char}. In {user}'s messages \"I\", \"me\" and \"my\" mean \
{user}, and \"you\" usually means {char}. Your own messages are {char}'s side of the story.",
    },
    Instruction {
        key: RULES_GROUP,
        group: ROLES,
        title: "Who is who (group chat)",
        help: "First part of the rules sent with every group reply. {cast} is every member; {char} is whose reply this is.",
        placeholders: &["{user}", "{cast}", "{char}"],
        default: "Roles: the user plays {user}; you play {cast}, one reply at a time, and this reply is {char}'s. Every user-role \
message is written by {user}: everything in it, *actions* included, is what {user} says, does, thinks or feels - never one of the \
characters. In {user}'s messages \"I\", \"me\" and \"my\" mean {user}. Each of your earlier messages starts with the name of the \
character who spoke it.",
    },
    Instruction {
        key: RULES_COMMON,
        group: ROLES,
        title: "Actions, speech and system notes",
        help: "Follows the who-is-who part in every roleplay reply.",
        placeholders: &["{user}"],
        default: "Text in *asterisks* is action or narration and text in \"quotes\" is speech; both belong to whoever wrote that \
message. A message in the form [System note: ...] comes from the app, not from {user}: follow it without mentioning it.",
    },
    Instruction {
        key: RULES_GROUP_MEMBERS,
        group: ROLES,
        title: "The other group members",
        help: "Group replies: keeps the reply to its speaker. {others} are the other members.",
        placeholders: &["{others}", "{char}"],
        default: "Don't write lines or actions for {others}: they speak in their own turns, so end the reply where {char}'s part \
ends. Start it straight with {char}'s words or actions, without \"{char}:\" in front - the app labels every message itself.",
    },
    Instruction {
        key: RULES_USER_ONLY,
        group: ROLES,
        title: "Your character is yours alone",
        help: "Ends the rules in every roleplay reply. {actor} is the character (in a group: \"a character\").",
        placeholders: &["{user}", "{actor}"],
        default: "{user} belongs to the user alone: never write what {user} says, does, thinks or feels, never retell {user}'s \
actions as if {actor} did them, and don't repeat what {user} just did - react to it and move the scene forward.",
    },
    // ---- tags
    Instruction {
        key: EMOTION_TAG,
        group: TAGS,
        title: "Emotion tag",
        help: "Every reply of a character with an expression pack. {emotions} is the pack's list. Must keep the [emotion: ...] format.",
        placeholders: &["{char}", "{emotions}"],
        default: "At the very end of every reply, on its own, append a tag naming the single emotion {char} feels while saying it, \
chosen from exactly this list: {emotions}. Format: [emotion: <one of the list above>]. Always include it, never explain it.",
    },
    Instruction {
        key: TIME_SKIP_TAG,
        group: TAGS,
        title: "Time-skip tag",
        help: "Every roleplay reply while the story has a clock. Must keep the [time-skip: ...] format.",
        placeholders: &[],
        default: "The story has a clock that only moves when you move it. At the very end of every reply, on its own last line, \
append a tag saying how much in-story time that reply covered - never omit it, and never write 0: a short line of dialogue or a quick \
reaction is about 1 minute; a described action, a wait or some small talk is several minutes; a meal, a drive or a long conversation is \
tens of minutes; an explicit time jump (an overnight, a training montage, \"the rest of the evening\") must match what was narrated. \
Format: [time-skip: <number> <minutes|hours|days>] with a single number, e.g. [time-skip: 2 minutes], [time-skip: 45 minutes] or \
[time-skip: 3 hours]. Do not explain the tag.",
    },
    // ---- photos
    Instruction {
        key: PHOTO,
        group: PHOTOS,
        title: "Characters send photos",
        help: "Every roleplay reply, while \"Draw the photos characters send\" is on in the picture settings. Must keep the [photo: ...] line format for the app to find the photo.",
        placeholders: &["{char}", "{user}"],
        default: "{char} sends {user} photos whenever a real person would: a selfie to show how {char} looks or what {char} is wearing, a \
snapshot of where {char} is, what {char} is doing or something {char} wants {user} to see - above all while they are apart and texting, \
and whenever {user} asks for one. When they are together, {char} can take a picture of the moment or of the two of them. When {char} \
sends or takes a photo in this reply, show it happening in the story and end the reply with one last line in exactly this form, square \
brackets included: [photo: <what the photo shows, as a camera would see it: who is in it, their pose, expression and what they are \
wearing (if anything), the place and the lighting>]. Write the description only inside the brackets, not in the story text. Leave the \
line out when no photo is sent or taken.",
    },
    Instruction {
        key: PHOTO_INTIMATE,
        group: PHOTOS,
        title: "Intimate photos",
        help: "Added after the photo text while \"Characters can send intimate photos\" is on - never for a character whose birth date makes them under 18 at the story's date.",
        placeholders: &["{char}", "{user}"],
        default: "The photos can be as intimate as the moment between them: when the mood is flirty or sexual and {char} wants to, {char} \
can send {user} suggestive or nude photos of {char}, described plainly.",
    },
    Instruction {
        key: USER_PHOTO,
        group: PHOTOS,
        title: "Photos you take",
        help: "Added when your latest message contains one of the photo words, while \"Draw the photos you take\" is on. Must keep the [user-photo: ...] line format.",
        placeholders: &["{user}"],
        default: "If {user} takes a photo or picture in {user}'s latest message, also describe that photo, as one more last line in exactly \
this form, square brackets included: [user-photo: <what {user}'s photo shows, as the camera sees it: who or what is in it (anyone by \
name), their pose, expression and what they are wearing (if anything), the place and the lighting>]. Leave this line out when {user} \
takes no photo.",
    },
    // ---- pictures the user sends
    Instruction {
        key: PICTURE_SHOWN,
        group: PICTURES,
        title: "A picture you're showing (roleplay)",
        help: "When your latest roleplay message has pictures attached. {what} is \"a picture\" or \"2 pictures\".",
        placeholders: &["{what}"],
        default: "The user's latest message includes {what}: something they are showing the character(s). Look at it and react in \
character to what it actually shows.",
    },
    Instruction {
        key: PICTURE_SHOWN_ASSISTANT,
        group: PICTURES,
        title: "A picture you're showing (assistant)",
        help: "When your latest assistant message has pictures attached.",
        placeholders: &["{what}"],
        default: "The user's latest message includes {what}. Look at it and answer with what it actually shows.",
    },
    Instruction {
        key: PICTURE_NOTE_ONE,
        group: PICTURES,
        title: "An earlier picture (described)",
        help: "Stands in for a picture on an older message of yours once it has a description, so it isn't sent again every reply.",
        placeholders: &["{description}"],
        default: "The user sent a picture with this message. It showed: {description}",
    },
    Instruction {
        key: PICTURE_NOTE_MANY,
        group: PICTURES,
        title: "Earlier pictures (described)",
        help: "The same for several pictures. {descriptions} is \"Picture 1: ... Picture 2: ...\".",
        placeholders: &["{count}", "{descriptions}"],
        default: "The user sent {count} pictures with this message. {descriptions}",
    },
    // ---- group chats
    Instruction {
        key: NEXT_SPEAKER,
        group: GROUPS,
        title: "Several members answering",
        help: "Group replies, in groups with \"Let several characters answer one message\" on. {others} are the members who haven't answered yet. Must keep the [next: <name>] line.",
        placeholders: &["{char}", "{user}", "{others}"],
        default: "Several members may answer the same message of {user}'s. Leave it at {char}'s reply when that answers the moment. Only \
when {others} would naturally speak up right after it (they were addressed too, have a strong reaction, or {char}'s reply calls for one), \
end the reply with one last line in exactly this form, square brackets included: [next: <name>], naming exactly one of {others}. Never \
write that reply yourself: they answer in their own turn.",
    },
    Instruction {
        key: PRIVATE,
        group: GROUPS,
        title: "Taking you aside",
        help: "Every new group reply, in groups with \"Let characters take you aside for a private talk\" on. Must keep the [private] line.",
        placeholders: &["{char}", "{user}"],
        default: "{char} can take {user} aside whenever that would come naturally: to say something not meant for the others' ears (a \
remark about someone, a secret, a worry, a confession, a bit of flirting, a plan) or just for a moment alone with {user}. Then show \
{char} drawing {user} aside, keep the private part itself for later, and end the reply with one last line in exactly this form, square \
brackets included: [private]. The app then takes {char} and {user} to a private conversation, where {char} goes on. Leave the line out \
when everything {char} wants to say can be said in front of the others.",
    },
    Instruction {
        key: GROUP_TURN,
        group: GROUPS,
        title: "Whose turn it is",
        help: "The very last note of every group reply's request, so the model plans only this member's reply.",
        placeholders: &["{char}", "{others}"],
        default: "Now write {char}'s reply, and only {char}'s. Any reply from {others} comes in a message of its own, so don't plan, think \
through or write their replies: think only about what {char} says and does next.",
    },
    // ---- private talks
    Instruction {
        key: PRIVATE_RULES,
        group: TALKS,
        title: "Inside a private talk",
        help: "Every reply in a private talk: who is there, and what the group-chat notes in its history are.",
        placeholders: &["{user}", "{char}", "{group}", "{others}"],
        default: "This is a private conversation between {user} and {char}, away from the group chat \"{group}\" with {others}. Nobody \
else can hear it. Notes in the form [System note: In the group chat, ...] are what was said in the group chat itself, where {char} was \
present; they are not part of this private conversation, so don't answer them here.",
    },
    Instruction {
        key: PRIVATE_MEMORY,
        group: TALKS,
        title: "Remembering a private talk",
        help: "A member's group replies after a private talk with you. The talk itself (or its summary) follows the text.",
        placeholders: &["{char}", "{user}"],
        default: "{char} and {user} have talked in private, away from the group. Only the two of them know what was said: {char} \
remembers it, but the others don't know it, and {char} doesn't bring it up in front of them unless {char} would choose to share it.",
    },
    Instruction {
        key: GROUP_NOTE,
        group: TALKS,
        title: "A group message, seen from a private talk",
        help: "How each group-chat message appears in a private talk's history (as a system note).",
        placeholders: &["{name}", "{text}"],
        default: "In the group chat, {name}: {text}",
    },
    Instruction {
        key: GROUP_EVENT,
        group: TALKS,
        title: "A group note, seen from a private talk",
        help: "How a note in the group chat (someone stepping away) appears in a private talk's history.",
        placeholders: &["{text}"],
        default: "In the group chat: {text}",
    },
    Instruction {
        key: STEPPED_AWAY,
        group: TALKS,
        title: "Stepped-away note",
        help: "The note the group chat gets where a private talk happens (shown in the chat, and read by the model as a system note). Changing it affects new notes only.",
        placeholders: &["{user}", "{char}"],
        default: "{user} and {char} stepped away from the group to talk in private.",
    },
    Instruction {
        key: GROUP_SUMMARY,
        group: TALKS,
        title: "The group's summary",
        help: "In a private talk whose group chat has been compacted.",
        placeholders: &["{summary}"],
        default: "What happened earlier in the group chat (a summary; older group messages are no longer shown):\n{summary}",
    },
    // ---- story clock and world
    Instruction {
        key: WORLD_NOW,
        group: WORLD,
        title: "Date, time and weather",
        help: "First line of the world-state block while the story has a clock.",
        placeholders: &["{when}", "{weather}"],
        default: "Current in-story date/time: {when}. Current weather: {weather}. Keep the narration consistent with this date/time and \
weather unless the story explicitly changes it.",
    },
    Instruction {
        key: WORLD_PLACE,
        group: WORLD,
        title: "Where the story is set",
        help: "When the story has a country.",
        placeholders: &["{place}"],
        default: "The story is set in {place}.",
    },
    Instruction {
        key: WORLD_HOLIDAY,
        group: WORLD,
        title: "Public holiday today",
        help: "On a public holiday.",
        placeholders: &["{names}"],
        default: "Today is a public holiday: {names}. Most people have the day off.",
    },
    Instruction {
        key: WORLD_OBSERVANCE,
        group: WORLD,
        title: "Observance today",
        help: "On a holiday that isn't a day off.",
        placeholders: &["{names}"],
        default: "Today is {names} (not a day off).",
    },
    Instruction {
        key: WORLD_WEEKEND,
        group: WORLD,
        title: "Weekend",
        help: "On Saturdays and Sundays without a holiday.",
        placeholders: &[],
        default: "It is the weekend.",
    },
    Instruction {
        key: WORLD_WEEKDAY,
        group: WORLD,
        title: "Weekday",
        help: "On other days, when the story has a country or a character has a routine.",
        placeholders: &[],
        default: "It is a regular weekday.",
    },
    Instruction {
        key: WORLD_UPCOMING,
        group: WORLD,
        title: "Upcoming holidays",
        help: "Holidays in the next days.",
        placeholders: &["{list}"],
        default: "Upcoming holidays: {list}.",
    },
    Instruction {
        key: WORLD_SCHOOL_OUT,
        group: WORLD,
        title: "School holidays now",
        help: "During school holidays.",
        placeholders: &["{name}", "{end}"],
        default: "School holidays: {name} until {end}. Schools are closed; students are free and school-day routines do not apply.",
    },
    Instruction {
        key: WORLD_SCHOOL_SOON,
        group: WORLD,
        title: "School holidays soon",
        help: "Before school holidays.",
        placeholders: &["{name}", "{start}", "{end}"],
        default: "School holidays start soon: {name} from {start} to {end}.",
    },
    Instruction {
        key: WORLD_ROUTINES,
        group: WORLD,
        title: "Daily routines",
        help: "When characters have schedules. {list} is one line per character.",
        placeholders: &["{list}"],
        default: "Daily routines (where each character's schedule has them right now; the story may override it): {list}",
    },
    Instruction {
        key: WORLD_METERS,
        group: WORLD,
        title: "Arousal meters",
        help: "When the story keeps meters.",
        placeholders: &["{list}"],
        default: "Arousal meters (kept by the story; continue from these): {list}",
    },
    Instruction {
        key: WORLD_BIRTHDAYS,
        group: WORLD,
        title: "Birthdays",
        help: "When characters or your persona have a birth date.",
        placeholders: &["{list}"],
        default: "Birthdays: {list}",
    },
    // ---- cards, persona and history
    Instruction {
        key: PERSONA_LABEL,
        group: CARDS,
        title: "Your persona",
        help: "The preset's persona block, when your persona has a description.",
        placeholders: &["{user}", "{description}"],
        default: "{user}'s persona: {description}",
    },
    Instruction {
        key: PERSONALITY_LABEL,
        group: CARDS,
        title: "A character's personality",
        help: "Follows a card's description in the preset's character block.",
        placeholders: &["{char}", "{personality}"],
        default: "{char}'s personality: {personality}",
    },
    Instruction {
        key: CAST_LABEL,
        group: CARDS,
        title: "Card fields in a group",
        help: "With several characters, each card's description, scenario or example dialogue is labelled with whose it is. {field} is \"description\", \"scenario\" or \"example dialogue\".",
        placeholders: &["{char}", "{field}", "{text}"],
        default: "{char}'s {field}: {text}",
    },
    Instruction {
        key: MEMORIES_HEADER,
        group: CARDS,
        title: "What the assistant remembers about you",
        help: "The preset's memory block in assistant chats. {memories} is one \"- fact\" line each.",
        placeholders: &["{memories}"],
        default: "What you remember about the user from past conversations:\n{memories}",
    },
    Instruction {
        key: SUMMARY_STORY,
        group: CARDS,
        title: "Summary of a compacted roleplay",
        help: "Stands in for the older messages of a compacted roleplay.",
        placeholders: &["{summary}"],
        default: "The story so far (older messages are no longer shown; this summary replaces them):\n{summary}",
    },
    Instruction {
        key: SUMMARY_ASSISTANT,
        group: CARDS,
        title: "Summary of a compacted assistant chat",
        help: "Stands in for the older messages of a compacted assistant chat.",
        placeholders: &["{summary}"],
        default: "Summary of the earlier conversation (older messages are no longer shown; this summary replaces them):\n{summary}",
    },
    // ---- continue, impersonate, assistant
    Instruction {
        key: CONTINUE_NUDGE,
        group: OTHER,
        title: "Continue",
        help: "The note Continue appends (as a system note) so the model extends the last reply instead of restarting it.",
        placeholders: &[],
        default: "Continue your last message without repeating its original content.",
    },
    Instruction {
        key: IMPERSONATE,
        group: OTHER,
        title: "Impersonate",
        help: "Asks the model to write your next message for you. {others} are the characters (or \"the assistant\").",
        placeholders: &["{user}", "{others}"],
        default: "This time, write {user}'s next message instead of a reply. {user} is the user's character, so for this one message the \
sides are swapped: write only what {user} says and does, from {user}'s point of view, matching the person, tense and style of {user}'s \
earlier messages. Don't write anything {others} says or does, and don't continue the last reply. Reply with only the message text, \
nothing else.",
    },
    Instruction {
        key: POWERSHELL_TOOL,
        group: OTHER,
        title: "PowerShell tool",
        help: "Assistant chats with the PowerShell tool on. Must keep the <powershell-run> tag format.",
        placeholders: &[],
        default: "You have access to a PowerShell 7 tool on the user's Windows machine. To run a command, reply with EXACTLY this and \
nothing else - no code fence, no explanation before or after it: <powershell-run>your command here</powershell-run>. If the command \
needs Administrator rights, add the elevate attribute: <powershell-run elevate=\"true\">your command here</powershell-run> - the user \
will be asked to approve elevation separately, every time, so only set it when the command genuinely requires it. After you emit the \
tag, stop: its real output is given back to you as the next message, and you then continue the conversation using that real output. \
Never invent or guess what a command would print. Only request a command when it's actually necessary to help the user - most replies \
need no tool at all.",
    },
];

/// The words that mark a message of yours as possibly taking a photo.
pub const DEFAULT_PHOTO_WORDS: &[&str] =
    &["photo", "photograph", "picture", "pic", "pics", "selfie", "snapshot", "snap", "snaps", "snapped", "camera", "shot of"];

/// The user's wording and switches for the app's own instructions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Instructions {
    /// Reworded texts by key; a missing or blank one uses the built-in text.
    pub texts: BTreeMap<String, String>,
    /// Ask replies to describe photos the user's character takes, and draw them.
    pub user_photos: bool,
    /// Let characters send suggestive or nude photos of themselves.
    pub intimate_photos: bool,
    /// Words in the user's message that may mean a photo is taken.
    pub photo_words: Vec<String>,
}

impl Default for Instructions {
    fn default() -> Self {
        Self {
            texts: BTreeMap::new(),
            user_photos: true,
            intimate_photos: true,
            photo_words: DEFAULT_PHOTO_WORDS.iter().map(|w| w.to_string()).collect(),
        }
    }
}

/// The built-in instructions, for code that has no settings at hand (and tests).
pub fn defaults() -> &'static Instructions {
    static DEFAULTS: LazyLock<Instructions> = LazyLock::new(Instructions::default);
    &DEFAULTS
}

impl Instructions {
    /// The text for `key` - the user's, else the built-in one - with its placeholders filled in.
    pub fn render(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let template = self
            .texts
            .get(key)
            .map(String::as_str)
            .filter(|t| !t.trim().is_empty())
            .or_else(|| ALL.iter().find(|i| i.key == key).map(|i| i.default))
            .unwrap_or_default();
        // One pass, so a value that happens to contain "{user}" isn't filled again.
        static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([a-z]+)\}").unwrap());
        PLACEHOLDER
            .replace_all(template.trim(), |caps: &regex::Captures| {
                vars.iter().find(|(name, _)| *name == &caps[1]).map_or_else(|| caps[0].to_string(), |(_, value)| value.to_string())
            })
            .into_owned()
    }

    /// `text` in the system-note wrapper: something the app slips into the history as a user turn.
    pub fn note(&self, text: &str) -> String {
        self.render(SYSTEM_NOTE, &[("text", text.trim())])
    }

    /// Whether `text` contains one of the photo words, as a whole word (any case).
    pub fn mentions_a_photo(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.photo_words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).any(|word| {
            lower.match_indices(&word).any(|(at, _)| {
                let before = lower[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
                let after = lower[at + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                before && after
            })
        })
    }

    /// Drops texts equal to the built-in ones (so a later default change still reaches them) and
    /// blank or unknown keys.
    fn cleaned(mut self) -> Self {
        self.texts.retain(|key, text| {
            ALL.iter().find(|i| i.key == key).is_some_and(|i| !text.trim().is_empty() && text.trim() != i.default.trim())
        });
        self.photo_words.retain(|w| !w.trim().is_empty());
        self
    }
}

pub fn load(conn: &Connection) -> Result<Instructions> {
    let Some(json) = app_settings::get(conn, SETTINGS_KEY)?.filter(|j| !j.is_empty()) else {
        return Ok(Instructions::default());
    };
    // Unreadable settings fall back to the built-in ones rather than breaking every reply.
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

pub fn save(conn: &Connection, instructions: Instructions) -> Result<Instructions> {
    let instructions = instructions.cleaned();
    app_settings::set(conn, SETTINGS_KEY, &serde_json::to_string(&instructions)?)?;
    Ok(instructions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn a_reworded_text_is_used_and_a_blank_one_falls_back() {
        let mut ins = Instructions::default();
        let vars = [("char", "Nova"), ("user", "Alex")];
        assert!(ins.render(PHOTO, &vars).starts_with("Nova sends Alex photos whenever a real person would"));
        ins.texts.insert(PHOTO.into(), "{char} texts {user} pictures. [photo: ...]".into());
        assert_eq!(ins.render(PHOTO, &vars), "Nova texts Alex pictures. [photo: ...]");
        ins.texts.insert(PHOTO.into(), "   ".into());
        assert!(ins.render(PHOTO, &vars).starts_with("Nova sends Alex photos"));
        assert_eq!(ins.render("nonsense", &vars), "");
    }

    #[test]
    fn values_are_filled_once_and_unknown_placeholders_stay() {
        let ins = Instructions::default();
        assert_eq!(ins.render(GROUP_NOTE, &[("name", "{text}"), ("text", "hi")]), "In the group chat, {text}: hi");
        assert_eq!(ins.note("  Remember the twist.\n"), "[System note: Remember the twist.]");
    }

    #[test]
    fn every_default_uses_exactly_its_listed_placeholders() {
        let placeholder = Regex::new(r"\{[a-z]+\}").unwrap();
        let mut keys = std::collections::HashSet::new();
        for i in ALL {
            assert!(keys.insert(i.key), "{} is listed twice", i.key);
            let used: std::collections::BTreeSet<&str> = placeholder.find_iter(i.default).map(|m| m.as_str()).collect();
            let listed: std::collections::BTreeSet<&str> = i.placeholders.iter().copied().collect();
            assert_eq!(used, listed, "{}", i.key);
        }
    }

    #[test]
    fn photo_words_match_whole_words_only() {
        let mut ins = Instructions::default();
        assert!(ins.mentions_a_photo("*Soso decides to make a picture of the harbour*"));
        assert!(ins.mentions_a_photo("I SNAP a selfie"));
        assert!(!ins.mentions_a_photo("We walk to the epicentre."), "'pic' inside a word");
        ins.photo_words = vec!["foto".into(), " ".into()];
        assert!(ins.mentions_a_photo("macht ein Foto"));
        assert!(!ins.mentions_a_photo("a picture"));
    }

    #[test]
    fn saving_keeps_only_real_changes() {
        let conn = open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), Instructions::default());
        let mut ins = Instructions { intimate_photos: false, ..Default::default() };
        let photo = ALL.iter().find(|i| i.key == PHOTO).unwrap();
        ins.texts.insert(PHOTO.into(), photo.default.into());
        ins.texts.insert(PRIVATE.into(), "{char} may whisper. [private]".into());
        ins.texts.insert("unknown".into(), "x".into());
        let saved = save(&conn, ins).unwrap();
        assert_eq!(saved.texts.keys().collect::<Vec<_>>(), ["private"], "the unchanged and unknown ones are dropped");
        assert_eq!(load(&conn).unwrap(), saved);
        assert!(!saved.intimate_photos);
    }
}
