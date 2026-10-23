// Preset values for the quick appearance builder. Values are written so they read naturally in the
// rendered text (src-tauri/src/prompting/appearance.rs), e.g. build "slim" -> "with a slim build",
// hips "wide" -> "wide hips". Every group also accepts a custom value.

import type { Appearance, Sex } from "./types";

export interface Choice {
  value: string;
  /** Colour swatch shown on the chip. */
  swatch?: string;
}

const plain = (...values: string[]): Choice[] => values.map((value) => ({ value }));

export const HAIR_COLORS: Choice[] = [
  { value: "black", swatch: "#1c1c1c" },
  { value: "dark brown", swatch: "#3b2417" },
  { value: "brown", swatch: "#6b4226" },
  { value: "light brown", swatch: "#a67b5b" },
  { value: "auburn", swatch: "#922724" },
  { value: "red", swatch: "#b7410e" },
  { value: "ginger", swatch: "#d2691e" },
  { value: "strawberry blonde", swatch: "#e3a869" },
  { value: "blonde", swatch: "#e6c87a" },
  { value: "platinum blonde", swatch: "#ece6d0" },
  { value: "grey", swatch: "#9e9e9e" },
  { value: "white", swatch: "#f4f4f4" },
  { value: "blue", swatch: "#3b6fd8" },
  { value: "pink", swatch: "#e58fb5" },
  { value: "purple", swatch: "#8a4fbf" },
  { value: "green", swatch: "#3f9a5a" },
];

export const EYE_COLORS: Choice[] = [
  { value: "dark brown", swatch: "#3a2412" },
  { value: "brown", swatch: "#5b3a1e" },
  { value: "hazel", swatch: "#8e7618" },
  { value: "amber", swatch: "#c68e17" },
  { value: "green", swatch: "#4f8a3c" },
  { value: "blue", swatch: "#3c78c4" },
  { value: "light blue", swatch: "#8fbbe8" },
  { value: "grey", swatch: "#8a949c" },
  { value: "violet", swatch: "#7d5ba6" },
  { value: "red", swatch: "#b22222" },
];

export const SKIN_TONES: Choice[] = [
  { value: "porcelain", swatch: "#f6e3d4" },
  { value: "fair", swatch: "#f1d3bc" },
  { value: "light", swatch: "#e5bf9f" },
  { value: "medium", swatch: "#c99772" },
  { value: "olive", swatch: "#b08d5e" },
  { value: "tan", swatch: "#a87554" },
  { value: "brown", swatch: "#8a5a3c" },
  { value: "dark brown", swatch: "#5c3a24" },
  { value: "deep", swatch: "#3d2416" },
];

export const HAIR_LENGTHS = plain("shaved", "very short", "short", "chin-length", "shoulder-length", "long", "very long");
export const HAIR_STYLES = plain(
  "straight",
  "wavy",
  "curly",
  "coily",
  "messy",
  "braided",
  "ponytail",
  "bun",
  "pigtails",
  "bob cut",
  "pixie cut",
  "slicked back",
  "undercut",
  "mohawk",
  "dreadlocks",
);

export const BUILDS: Record<Sex, Choice[]> = {
  female: plain("petite", "slender", "slim", "average", "athletic", "curvy", "hourglass", "voluptuous", "plus-size"),
  male: plain("lean", "slim", "average", "athletic", "muscular", "stocky", "broad", "heavyset", "plus-size"),
};
export const MUSCLE_TONES = plain("soft", "average", "toned", "muscular", "very muscular");
export const BODY_HAIR = plain("smooth", "light", "moderate", "hairy");
export const MARKS = plain("freckles", "glasses", "tattoos", "piercings", "a scar", "a beauty mark", "dimples", "a birthmark");

export const CUP_SIZES = plain("AA", "A", "B", "C", "D", "DD", "E", "F", "G", "H");
export const BUSTS = plain("small", "perky", "round", "full", "heavy");
export const WAISTS = plain("narrow", "slim", "average", "soft", "thick");
export const HIPS = plain("narrow", "average", "wide", "very wide");
export const BUTTS = plain("small", "flat", "toned", "round", "full", "large");

export const SHOULDERS = plain("narrow", "average", "broad", "very broad");
export const CHESTS = plain("slim", "flat", "toned", "muscular", "broad", "barrel");
export const FACIAL_HAIR = plain("clean-shaven", "stubble", "mustache", "goatee", "short beard", "full beard", "long beard");

export const NIPPLES = plain("small", "large", "puffy", "pink", "light brown", "dark", "pierced");
export const PUBIC_HAIR = plain("shaved", "trimmed", "landing strip", "natural", "thick");
export const GENITALS: Record<Sex, Choice[]> = {
  female: plain("petite", "average", "full", "pierced"),
  male: plain("small", "average", "above average", "large", "very large"),
};
export const CIRCUMCISION = plain("circumcised", "uncircumcised");

/** Average adult height (cm) used for defaults and Randomize. */
export const AVERAGE_HEIGHT: Record<Sex, number> = { female: 165, male: 178 };

/** Rough body-mass index per build, so Randomize picks a weight that fits the build. */
const BMI: Record<string, number> = {
  petite: 19, slender: 18.5, slim: 19.5, lean: 20.5, average: 22.5, athletic: 22.5, curvy: 24, hourglass: 23,
  voluptuous: 26, muscular: 26, stocky: 27, broad: 26, heavyset: 30, "plus-size": 32,
};

export function emptyAppearance(sex: Sex | null = null): Appearance {
  return {
    sex,
    heightCm: null,
    weightKg: null,
    build: null,
    muscleTone: null,
    skinTone: null,
    hairColor: null,
    hairLength: null,
    hairStyle: null,
    eyeColor: null,
    bodyHair: null,
    marks: [],
    female: { cupSize: null, bust: null, waist: null, hips: null, butt: null },
    male: { shoulders: null, chest: null, facialHair: null },
    adult: { nipples: null, pubicHair: null, genitals: null, circumcision: null, notes: null },
    notes: null,
  };
}

/** True when nothing has been picked (the sex alone doesn't count). */
export function isEmptyAppearance(a: Appearance | null): boolean {
  const filled = (v: unknown): boolean =>
    Array.isArray(v)
      ? v.some(filled)
      : v !== null && typeof v === "object"
        ? Object.values(v).some(filled)
        : typeof v === "string"
          ? v.trim() !== ""
          : typeof v === "number" && v > 0;
  return !a || !filled({ ...a, sex: null });
}

const pick = <T>(items: T[]): T => items[Math.floor(Math.random() * items.length)];
const values = (choices: Choice[]) => choices.map((c) => c.value);
const chance = (p: number) => Math.random() < p;

/** Plausible random picks for one-click creation. Keeps the chosen sex (random if none) and any
 * picks already set at the end of the form. */
export function randomAppearance(current: Appearance | null): Appearance {
  const sex: Sex = current?.sex ?? pick<Sex>(["female", "male"]);
  const a = emptyAppearance(sex);
  if (current) a.adult = { ...current.adult }; // `current` may be a Svelte state proxy, which structuredClone rejects
  a.build = pick(values(BUILDS[sex]));
  a.muscleTone = pick(["soft", "average", "toned", ...(a.build === "muscular" || a.build === "athletic" ? ["muscular"] : [])]);
  a.heightCm = Math.round(AVERAGE_HEIGHT[sex] + (Math.random() + Math.random() + Math.random() - 1.5) * 14);
  a.weightKg = Math.round((BMI[a.build] ?? 22.5) * (a.heightCm / 100) ** 2);
  a.skinTone = pick(values(SKIN_TONES));
  a.hairColor = pick(values(HAIR_COLORS).slice(0, 12)); // natural colours only
  a.eyeColor = pick(values(EYE_COLORS).slice(0, 8));
  a.hairLength = sex === "female" ? pick(["chin-length", "shoulder-length", "long", "long", "very long", "short"]) : pick(["very short", "short", "short", "chin-length", "shoulder-length", "long"]);
  a.hairStyle = pick(["straight", "wavy", "curly", "messy", ...(sex === "female" ? ["ponytail", "braided", "bun"] : ["slicked back", "undercut"])]);
  a.bodyHair = sex === "female" ? pick(["smooth", "smooth", "light"]) : pick(values(BODY_HAIR));
  if (chance(0.35)) a.marks = [pick(values(MARKS))];
  if (sex === "female") {
    a.female = { cupSize: pick(values(CUP_SIZES).slice(1, 8)), bust: pick(values(BUSTS)), waist: pick(["narrow", "slim", "average"]), hips: pick(values(HIPS).slice(0, 3)), butt: pick(values(BUTTS)) };
  } else {
    a.male = { shoulders: pick(values(SHOULDERS).slice(1)), chest: pick(values(CHESTS)), facialHair: pick(values(FACIAL_HAIR)) };
  }
  return a;
}

export const cmToFeetInches = (cm: number) => {
  const inches = Math.round(cm / 2.54);
  return `${Math.floor(inches / 12)}'${inches % 12}"`;
};
export const kgToPounds = (kg: number) => Math.round(kg * 2.20462);
