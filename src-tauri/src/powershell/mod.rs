//! The assistant's PowerShell tool, minus the chat loop: its options, running a command, and
//! catching the tool-call tag while a reply streams. (The wire protocol lives in
//! `prompting::powershell_tool`; the loop that ties it together is in `chat::engine`.)

pub mod exec;
pub mod filter;
pub mod settings;
