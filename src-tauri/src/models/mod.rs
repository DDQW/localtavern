//! Domain models. They serialize to camelCase for the frontend; the SQLite layer stores enums as
//! their raw int value (see `int_enum!`) exactly like the original C# app, so an existing member's
//! number must never change - append only.

/// Generates `to_i32`/`from_i32` for a fieldless enum so it can be persisted as an integer column.
macro_rules! int_enum {
    ($name:ident { $($variant:ident = $val:expr),+ $(,)? }) => {
        impl $name {
            pub fn to_i32(self) -> i32 {
                match self { $(Self::$variant => $val),+ }
            }
            pub fn from_i32(v: i32) -> Option<Self> {
                match v { $($val => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

pub mod appearance;
pub mod character;
pub mod chat;
pub mod conversation;
pub mod group_chat;
pub mod lorebook;
pub mod misc;
pub mod persona;
pub mod prompt_preset;
pub mod schedule;
pub mod story_character;

pub use appearance::*;
pub use character::*;
pub use chat::*;
pub use conversation::*;
pub use group_chat::*;
pub use lorebook::*;
pub use misc::*;
pub use persona::*;
pub use prompt_preset::*;
pub use schedule::*;
pub use story_character::*;
