<script lang="ts">
  import { page } from "$app/state";
  import "../app.css";
  import Lightbox from "$lib/components/Lightbox.svelte";
  import { ui } from "$lib/ui.svelte";

  let { children } = $props();

  const items = [
    { href: "/", label: "Assistant", icon: "💬" },
    { href: "/roleplay", label: "Roleplay", icon: "🎭" },
    { href: "/characters", label: "Characters", icon: "🧙" },
    { href: "/personas", label: "Personas", icon: "👤" },
    { href: "/lorebooks", label: "Lorebooks", icon: "📚" },
    { href: "/presets", label: "Prompt Presets", icon: "🧩" },
    { href: "/groups", label: "Group Chats", icon: "👥" },
    { href: "/history", label: "History", icon: "🕘" },
  ];

  const path = $derived(page.url.pathname);
  const active = (href: string) =>
    href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);
  // A character chat belongs under "Roleplay"; a conversation opened from History under neither.
  const activeItem = (href: string) =>
    href === "/roleplay" ? active(href) || path.startsWith("/character/") || path.startsWith("/group/") : active(href);

  // On a phone the menu is a drawer: going somewhere closes it.
  $effect(() => {
    path;
    ui.navOpen = false;
  });
</script>

<div class="app" class:nav-open={ui.navOpen}>
  <header class="topbar">
    <button class="ghost menu" onclick={() => (ui.navOpen = !ui.navOpen)} aria-label="Menu" aria-expanded={ui.navOpen}>☰</button>
    <span class="brand">LocalTavern</span>
  </header>

  <nav class="sidebar" aria-label="Main">
    <div class="brand">LocalTavern</div>

    <ul>
      {#each items as item (item.href)}
        <li>
          <a class="item" class:active={activeItem(item.href)} href={item.href} aria-current={activeItem(item.href) ? "page" : undefined}>
            <span class="icon" aria-hidden="true">{item.icon}</span>{item.label}
          </a>
        </li>
      {/each}
    </ul>

    <div class="grow"></div>
    <a class="item" class:active={active("/settings")} href="/settings">
      <span class="icon" aria-hidden="true">⚙️</span>Settings
    </a>
  </nav>

  {#if ui.navOpen}
    <button class="backdrop" aria-label="Close menu" onclick={() => (ui.navOpen = false)}></button>
  {/if}

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
    height: 100dvh;
    /* Android draws the app edge to edge, under the status and navigation bars. */
    padding: env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom) env(safe-area-inset-left);
  }

  .topbar {
    display: none;
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

  .item:hover {
    background: var(--panel-2);
  }

  .item.active {
    background: var(--panel-3);
    font-weight: 600;
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .icon {
    width: 20px;
    text-align: center;
  }

  .grow {
    flex: 1;
  }

  .main {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  /* Phone: a slim top bar, and the sidebar slides in over the page. */
  @media (max-width: 760px) {
    .app {
      grid-template-columns: 1fr;
      grid-template-rows: auto minmax(0, 1fr);
    }

    .topbar {
      display: flex;
      align-items: center;
      gap: 4px;
      padding: 2px 8px;
      background: var(--panel);
      border-bottom: 1px solid var(--border);
    }

    .topbar .brand {
      padding: 0;
    }

    .menu {
      font-size: 20px;
      padding: 4px 10px;
    }

    .sidebar {
      position: fixed;
      inset: 0 auto 0 0;
      width: min(260px, 80vw);
      z-index: 40;
      overflow-y: auto;
      padding-top: calc(env(safe-area-inset-top) + 12px);
      padding-bottom: calc(env(safe-area-inset-bottom) + 12px);
      transform: translateX(-100%);
      transition: transform 0.18s ease-out;
      box-shadow: var(--shadow);
    }

    .nav-open .sidebar {
      transform: none;
    }

    .item {
      padding: 11px 10px;
    }

    .backdrop {
      position: fixed;
      inset: 0;
      z-index: 35;
      padding: 0;
      border: 0;
      border-radius: 0;
      background: rgb(0 0 0 / 0.45);
    }
  }
</style>
