<script lang="ts">
  // Sound effects and spoken lines from ElevenLabs. Replies carry them as play buttons; the audio is
  // only made - and only costs credits - the first time a button is pressed.
  import { api, errorText } from "#lib/api.js";
  import type { AudioSettings, Character, Voice } from "#lib/types.js";

  let settings = $state<AudioSettings | null>(null);
  let characters = $state<Character[]>([]);
  let voices = $state<Voice[]>([]);
  let hasKey = $state(false);
  let key = $state("");
  let status = $state("");
  let keyStatus = $state("");
  let loadingVoices = $state(false);

  $effect(() => {
    void api
      .getAudioSettings()
      .then((loaded) => (settings = loaded))
      .catch((e) => (status = errorText(e)));
    void api.audioHasKey().then((has) => (hasKey = has)).catch(() => (hasKey = false));
    void api.listCharacters().then((all) => (characters = all)).catch(() => (characters = []));
  });

  /** The picked voice, kept in the list even before the account's voices are loaded. */
  function withCurrent(current: string): Voice[] {
    return current && !voices.some((v) => v.id === current) ? [{ id: current, name: current, category: "" }, ...voices] : voices;
  }

  async function saveKey() {
    try {
      hasKey = await api.setAudioKey(key);
      key = "";
      keyStatus = hasKey ? "Key saved." : "Key removed.";
      if (hasKey) await loadVoices();
    } catch (e) {
      keyStatus = errorText(e);
    }
  }

  async function loadVoices() {
    loadingVoices = true;
    try {
      voices = await api.listVoices();
      status = `${voices.length} voice${voices.length === 1 ? "" : "s"} found.`;
    } catch (e) {
      status = errorText(e);
    } finally {
      loadingVoices = false;
    }
  }

  function setCharacterVoice(id: string, voice: string) {
    if (!settings) return;
    if (voice) settings.characterVoices[id] = voice;
    else delete settings.characterVoices[id];
  }

  async function save() {
    if (!settings) return;
    try {
      settings = await api.saveAudioSettings($state.snapshot(settings));
      status = "Saved.";
    } catch (e) {
      status = errorText(e);
    }
  }
</script>

<article class="card block" id="sounds">
  <h2>Sounds and voices</h2>
  <p class="muted small">
    Characters can add a sound effect or speak a line aloud when it fits the moment. Each shows up as a play button on the reply, and
    <strong>ElevenLabs only makes the audio, and only charges credits, the first time you press it.</strong> Needs an
    <a href="https://elevenlabs.io/app/settings/api-keys" target="_blank" rel="noreferrer">ElevenLabs API key</a>.
  </p>

  <div class="keyrow">
    <input
      type="password"
      bind:value={key}
      placeholder={hasKey ? "Key saved - paste a new one to replace it" : "ElevenLabs API key"}
      autocomplete="off"
      aria-label="ElevenLabs API key"
    />
    <button onclick={saveKey} disabled={!key.trim() && !hasKey}>{key.trim() || !hasKey ? "Save key" : "Remove key"}</button>
    {#if keyStatus}<span class="muted small" role="status">{keyStatus}</span>{/if}
  </div>

  {#if settings}
    <label class="check"><input type="checkbox" bind:checked={settings.sounds} /> Sound effects (door slams, thunder, …)</label>
    <label class="check"><input type="checkbox" bind:checked={settings.voices} /> Spoken lines (a voice message, a shout, a whispered warning, …)</label>

    <div class="grid">
      <label class="field">
        Sound effect length
        <select bind:value={settings.soundSeconds}>
          <option value={0}>Automatic (cheapest)</option>
          {#each [2, 3, 5, 10, 22] as seconds (seconds)}<option value={seconds}>{seconds} seconds</option>{/each}
        </select>
      </label>
      <label class="field">
        Speech model
        <input bind:value={settings.speechModel} placeholder="eleven_multilingual_v2" />
      </label>
    </div>

    <div class="keyrow">
      <button onclick={loadVoices} disabled={!hasKey || loadingVoices}>{loadingVoices ? "Loading…" : "Load my voices"}</button>
      {#if !hasKey}<span class="muted small">Save your key first.</span>{/if}
    </div>

    <label class="field">
      Voice for characters without their own
      <select bind:value={settings.defaultVoiceId}>
        <option value="">None (they don't speak aloud)</option>
        {#each withCurrent(settings.defaultVoiceId) as voice (voice.id)}
          <option value={voice.id}>{voice.name}{voice.category ? ` (${voice.category})` : ""}</option>
        {/each}
      </select>
    </label>

    {#if characters.length > 0}
      <div class="voices">
        <p class="muted small">Each character's own voice:</p>
        {#each characters as character (character.id)}
          {@const current = settings.characterVoices[character.id] ?? ""}
          <label class="voicerow">
            <span>{character.name}</span>
            <select value={current} onchange={(e) => setCharacterVoice(character.id, e.currentTarget.value)}>
              <option value="">Default voice</option>
              {#each withCurrent(current) as voice (voice.id)}
                <option value={voice.id}>{voice.name}{voice.category ? ` (${voice.category})` : ""}</option>
              {/each}
            </select>
          </label>
        {/each}
      </div>
    {/if}

    <div class="keyrow">
      <button class="primary" onclick={save}>Save</button>
      {#if status}<span class="muted small" role="status">{status}</span>{/if}
    </div>
  {/if}
</article>

<style>
  .block {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  h2 {
    font-size: 16px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .field select,
  .field input {
    color: var(--text);
    font-size: 14px;
  }

  .keyrow {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .keyrow input {
    flex: 1;
    min-width: 0;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .voices {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .voicerow {
    display: grid;
    grid-template-columns: minmax(90px, 1fr) minmax(160px, 2fr);
    align-items: center;
    gap: 10px;
    font-size: 14px;
  }

  .small {
    font-size: 12px;
  }
</style>
