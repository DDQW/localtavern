<script lang="ts">
  import { page } from "$app/state";
  import "../app.css";
  import Lightbox from "$lib/components/Lightbox.svelte";

  let { children } = $props();

  // `soon` items exist in the original app and are being ported; they stay visible so the layout is
  // the familiar one, but aren't clickable yet.
  const items = [
    { href: "/", label: "Assistant", icon: "💬" },
    { href: "/roleplay", label: "Roleplay", icon: "🎭" },
    { href: "/characters", label: "Characters", icon: "🧙" },
    { href: "/personas", label: "Personas", icon: "👤", soon: true },
    { href: "/lorebooks", label: "Lorebooks", icon: "📚", soon: true },
    { href: "/presets", label: "Prompt Presets", icon: "🧩", soon: true },
    { href: "/groups", label: "Group Chats", icon: "👥", soon: true },
    { href: "/history", label: "History", icon: "🕘" },
  ];

  const path = $derived(page.url.pathname);
  const active = (href: string) =>
    href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);
  // A character chat belongs under "Roleplay"; a conversation opened from History under neither.
  const activeItem = (href: string) => (href === "/roleplay" ? active(href) || path.startsWith("/character/") : active(href));
</script>

<div class="app">
  <nav class="sidebar" aria-label="Main">
    <div class="brand">LocalTavern</div>

    <ul>
      {#each items as item (item.href)}
        <li>
          {#if item.soon}
            <span class="item soon" title="Being ported from the original app">
              <span class="icon" aria-hidden="true">{item.icon}</span>{item.label}
              <span class="badge">soon</span>
            </span>
          {:else}
            <a class="item" class:active={activeItem(item.href)} href={item.href} aria-current={activeItem(item.href) ? "page" : undefined}>
              <span class="icon" aria-hidden="true">{item.icon}</span>{item.label}
            </a>
          {/if}
        </li>
      {/each}
    </ul>

    <div class="grow"></div>
    <a class="item" class:active={active("/settings")} href="/settings">
      <span class="icon" aria-hidden="true">⚙️</span>Settings
    </a>
  </nav>

  <main class="main">
    {@render children()}
  </main>
</div>

<Lightbox />

<style>
  .app {
    display: grid;
    grid-template-columns: 208px 1fr;
    height: 100vh;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    padding: 12px 10px;
    background: var(--panel);
    border-right: 1px solid var(--border);
    gap: 2px;
  }

  .brand {
    font-weight: 700;
    font-size: 15px;
    letter-spacing: 0.02em;
    padding: 6px 10px 14px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    color: var(--text);
    text-decoration: none;
    cursor: pointer;
  }

  .item:hover:not(.soon) {
    background: var(--panel-2);
  }

  .item.active {
    background: var(--panel-3);
    font-weight: 600;
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .item.soon {
    color: var(--muted);
    cursor: default;
    opacity: 0.6;
  }

  .icon {
    width: 20px;
    text-align: center;
  }

  .badge {
    margin-left: auto;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }

  .grow {
    flex: 1;
  }

  .main {
    min-width: 0;
    height: 100vh;
    overflow: hidden;
  }
</style>
