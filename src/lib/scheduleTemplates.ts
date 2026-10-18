// Starting points for a character's weekly schedule (src/lib/components/ScheduleSection.svelte).

import type { DayKind, DayPlan, Schedule, ScheduleBlock } from "./types";

export const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
export const DAY_KINDS: { value: DayKind; label: string }[] = [
  { value: "work", label: "Work" },
  { value: "school", label: "School" },
  { value: "shift", label: "Shift" },
  { value: "off", label: "Off" },
];

const block = (start: string, end: string, activity: string, place: string | null = null): ScheduleBlock => ({ start, end, activity, place });
const day = (kind: DayKind, ...blocks: ScheduleBlock[]): DayPlan => ({ kind, blocks });

export function emptySchedule(): Schedule {
  return {
    homeCountry: null,
    homeRegion: null,
    days: Array.from({ length: 7 }, () => day("off")),
    holiday: day("off"),
    observesHolidays: true,
    notes: null,
  };
}

const weekend = () => [day("off", block("10:00", "12:00", "brunch"), block("15:00", "18:00", "out with friends")), day("off", block("11:00", "13:00", "slow morning"), block("16:00", "18:00", "family time"))];

export const TEMPLATES: { name: string; build: () => Pick<Schedule, "days" | "holiday"> }[] = [
  {
    name: "Office 9–5",
    build: () => {
      const work = () => day("work", block("08:15", "09:00", "commute"), block("09:00", "17:00", "work", "the office"), block("19:00", "20:00", "dinner"));
      return { days: [work(), work(), work(), work(), work(), ...weekend()], holiday: day("off", block("12:00", "15:00", "holiday lunch with family")) };
    },
  },
  {
    name: "Student",
    build: () => {
      const school = () => day("school", block("07:30", "08:00", "way to school"), block("08:00", "15:00", "classes", "school"), block("16:00", "18:00", "homework"));
      return { days: [school(), school(), school(), school(), school(), ...weekend()], holiday: day("off", block("11:00", "17:00", "hanging out with friends")) };
    },
  },
  {
    name: "Night shift",
    build: () => {
      const shift = () => day("shift", block("13:00", "14:00", "late breakfast"), block("22:00", "06:00", "night shift", "the hospital"));
      return { days: [shift(), shift(), shift(), shift(), day("off"), ...weekend()], holiday: day("off") };
    },
  },
  {
    name: "Free / retired",
    build: () => {
      const free = () => day("off", block("09:00", "10:00", "breakfast"), block("14:00", "16:00", "walk"));
      return { days: Array.from({ length: 7 }, free), holiday: free() };
    },
  },
];
