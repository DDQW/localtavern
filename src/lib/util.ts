// Small helpers shared by the editor pages.

/** A v4 UUID for records created in the UI (the backend accepts any well-formed id). */
export function newId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export const nowIso = () => new Date().toISOString();

/** Splits "a, b,, c" into ["a", "b", "c"]. */
export function splitList(text: string): string[] {
  return text
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

/** An empty or invalid number input means "not set". */
export function numberOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** Moves item `index` by `delta` places in `list`, in place. No-op at the ends. */
export function move<T>(list: T[], index: number, delta: number) {
  const target = index + delta;
  if (target < 0 || target >= list.length) return;
  [list[index], list[target]] = [list[target], list[index]];
}
