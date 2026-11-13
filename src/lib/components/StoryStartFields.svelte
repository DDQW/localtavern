<script lang="ts">
  // Day and time as two controls. One datetime-local input sits in a flex row whose
  // inputs are allowed to shrink to nothing, and WebKit then won't edit its segments.
  let {
    value = null,
    onchange,
  }: {
    /** Local story clock, "YYYY-MM-DDTHH:MM:SS", or null when unset. */
    value: string | null;
    onchange: (next: string | null) => void;
  } = $props();

  let day = $state("");
  let clock = $state("");

  $effect(() => {
    day = value?.slice(0, 10) ?? "";
    clock = value && value.length >= 16 ? value.slice(11, 16) : "";
  });

  function commit(nextDay: string, nextClock: string) {
    day = nextDay;
    clock = nextClock;
    if (!nextDay) {
      onchange(null);
      return;
    }
    onchange(`${nextDay}T${nextClock || "09:00"}:00`);
  }
</script>

<div class="start">
  <label class="field">
    Story starts
    <input type="date" value={day} onchange={(e) => commit(e.currentTarget.value, clock)} />
  </label>
  <label class="field">
    Time
    <input type="time" value={clock} onchange={(e) => commit(day, e.currentTarget.value)} />
  </label>
</div>

<style>
  .start {
    display: flex;
    flex: 0 0 auto;
    flex-wrap: wrap;
    gap: 12px 16px;
    align-items: flex-end;
  }

  .start label {
    flex: 0 0 auto;
  }

  input[type="date"] {
    width: 11rem;
    min-width: 11rem;
  }

  input[type="time"] {
    width: 7.5rem;
    min-width: 7.5rem;
  }
</style>
