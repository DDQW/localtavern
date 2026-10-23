# LocalTavern

LocalTavern is a desktop app for chatting with AI characters and assistants. Chats, cards, and pictures stay on your machine. You bring your own model: a local one such as Ollama, or a cloud account.

It is aimed at the same kind of use as SillyTavern: roleplay with a character, group scenes, lore that shows up when it is relevant, and prompts you can rearrange.

## What you can do

- **Assistant and roleplay.** A plain assistant chat, or a story with a character, your persona, and a world clock (date, weather, holidays, routines).
- **Library.** Characters, personas, lorebooks, prompt presets, and group chats. Character cards import and export as JSON or as a PNG with the card embedded.
- **Groups.** Several characters in one chat, with private talks off to the side.
- **Pictures and video.** Characters can send photos and short clips in the story. Pictures are drawn through Automatic1111 or ComfyUI.
- **Models.** Ollama, a GGUF model run in the app, Gemini, DeepSeek, Mistral, ChatGPT, Grok, and a Vast.ai instance.

## Download

The Windows installer is on the [releases](https://github.com/DDQW/localtavern/releases) page:

[LocalTavern 0.2.27 for Windows (x64)](https://github.com/DDQW/localtavern/releases/download/v0.2.27/LocalTavern_0.2.27_x64-setup.exe)

It installs for the current user. Your data is kept in the app's own folder, not in the installer.

## Build from source

You need Node.js, Rust, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your system.

```sh
npm install
npm run tauri dev
```

A Windows installer (NSIS) is produced with:

```sh
npm run tauri build
```

The setup file is written to `src-tauri/target/release/bundle/nsis/`.

## License

GNU Affero General Public License v3.0
