use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

/// What kind of day a plan describes; named in prompts ("workday", "school day", ...).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DayKind {
    Work,
    School,
    Shift,
    #[default]
    Off,
}

impl DayKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Work => "workday",
            Self::School => "school day",
            Self::Shift => "shift day",
            Self::Off => "day off",
        }
    }
}

/// One timetable entry, `HH:MM` to `HH:MM`. An end before the start runs past midnight (a night
/// shift), covering the early hours of the next day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduleBlock {
    pub start: String,
    pub end: String,
    pub activity: String,
    pub place: Option<String>,
}

impl ScheduleBlock {
    fn minutes(&self) -> Option<(u32, u32)> {
        let parse = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok().map(|t| t.hour() * 60 + t.minute());
        Some((parse(&self.start)?, parse(&self.end)?)).filter(|_| !self.activity.trim().is_empty())
    }

    fn overnight(&self) -> bool {
        self.minutes().is_some_and(|(s, e)| e <= s)
    }

    /// "work (bakery)".
    pub fn describe(&self) -> String {
        match self.place.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(place) => format!("{} ({place})", self.activity.trim()),
            None => self.activity.trim().to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayPlan {
    pub kind: DayKind,
    pub blocks: Vec<ScheduleBlock>,
}

/// A character's weekly routine: one plan per weekday (Monday first) plus the plan for public
/// holidays, and where they live (the default setting for new roleplays).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Schedule {
    pub home_country: Option<String>,
    pub home_region: Option<String>,
    /// The in-story date and time new roleplays with this character begin at.
    pub story_start: Option<NaiveDateTime>,
    /// Monday to Sunday; missing days count as days off.
    pub days: Vec<DayPlan>,
    pub holiday: DayPlan,
    /// Use the holiday plan on public holidays instead of that weekday's plan.
    pub observes_holidays: bool,
    pub notes: Option<String>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            home_country: None,
            home_region: None,
            story_start: None,
            days: vec![DayPlan::default(); 7],
            holiday: DayPlan::default(),
            observes_holidays: true,
            notes: None,
        }
    }
}

/// What a schedule says about one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleNow<'a> {
    pub kind: DayKind,
    pub holiday: bool,
    pub current: Option<&'a ScheduleBlock>,
    pub next: Option<&'a ScheduleBlock>,
}

impl Schedule {
    /// Nothing filled in at all (no home, no start, no blocks, every day a plain day off).
    pub fn is_empty(&self) -> bool {
        let blank = |d: &DayPlan| d.kind == DayKind::Off && d.blocks.is_empty();
        self.home_country.as_deref().is_none_or(|c| c.trim().is_empty())
            && self.story_start.is_none()
            && self.days.iter().all(blank)
            && blank(&self.holiday)
            && self.notes.as_deref().is_none_or(|n| n.trim().is_empty())
    }

    fn weekday(&self, day: Weekday) -> Option<&DayPlan> {
        self.days.get(day.num_days_from_monday() as usize)
    }

    /// The plan in force on `date`; `is_holiday` says whether that date is a public day off.
    pub fn plan_for(&self, date: NaiveDate, is_holiday: bool) -> (Option<&DayPlan>, bool) {
        if is_holiday && self.observes_holidays {
            (Some(&self.holiday), true)
        } else {
            (self.weekday(date.weekday()), false)
        }
    }

    /// Where the routine has them at `date` `time`. Yesterday's overnight blocks reach into today's
    /// early hours.
    pub fn now(&self, date: NaiveDate, time: NaiveTime, is_holiday: &dyn Fn(NaiveDate) -> bool) -> ScheduleNow<'_> {
        let minute = time.hour() * 60 + time.minute();
        let (today, holiday) = self.plan_for(date, is_holiday(date));
        let blocks = today.map(|p| p.blocks.as_slice()).unwrap_or_default();

        let carried = date.pred_opt().and_then(|y| self.plan_for(y, is_holiday(y)).0).and_then(|plan| {
            plan.blocks.iter().find(|b| b.overnight() && b.minutes().is_some_and(|(_, end)| minute < end))
        });
        let current = carried.or_else(|| {
            blocks.iter().find(|b| match b.minutes() {
                Some((start, end)) if end > start => (start..end).contains(&minute),
                Some((start, _)) => minute >= start, // overnight: runs to midnight today
                None => false,
            })
        });
        let next = blocks
            .iter()
            .filter_map(|b| b.minutes().map(|(start, _)| (start, b)))
            .filter(|(start, _)| *start > minute)
            .min_by_key(|(start, _)| *start)
            .map(|(_, b)| b);

        ScheduleNow { kind: today.map(|p| p.kind).unwrap_or_default(), holiday, current, next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(start: &str, end: &str, activity: &str, place: Option<&str>) -> ScheduleBlock {
        ScheduleBlock { start: start.into(), end: end.into(), activity: activity.into(), place: place.map(String::from) }
    }

    fn worker() -> Schedule {
        let workday = DayPlan {
            kind: DayKind::Work,
            blocks: vec![block("08:00", "16:00", "work", Some("bakery")), block("18:00", "19:30", "gym", None)],
        };
        let mut s = Schedule::default();
        for d in 0..5 {
            s.days[d] = workday.clone();
        }
        s.days[5].blocks = vec![block("22:00", "03:00", "clubbing", Some("the Lantern"))];
        s.holiday = DayPlan { kind: DayKind::Off, blocks: vec![block("12:00", "14:00", "family lunch", None)] };
        s
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    const NO_HOLIDAYS: &dyn Fn(NaiveDate) -> bool = &|_| false;

    #[test]
    fn finds_the_current_and_next_block_on_a_weekday() {
        let s = worker();
        let now = s.now(date(2025, 6, 2), at(10, 15), NO_HOLIDAYS); // a Monday
        assert_eq!(now.kind, DayKind::Work);
        assert_eq!(now.current.unwrap().describe(), "work (bakery)");
        assert_eq!(now.next.unwrap().activity, "gym");

        let evening = s.now(date(2025, 6, 2), at(16, 30), NO_HOLIDAYS);
        assert!(evening.current.is_none());
        assert_eq!(evening.next.unwrap().activity, "gym");
    }

    #[test]
    fn overnight_blocks_reach_into_the_next_morning() {
        let s = worker();
        assert_eq!(s.now(date(2025, 6, 7), at(23, 0), NO_HOLIDAYS).current.unwrap().activity, "clubbing"); // Saturday
        let sunday = s.now(date(2025, 6, 8), at(2, 30), NO_HOLIDAYS);
        assert_eq!(sunday.current.unwrap().activity, "clubbing");
        assert_eq!(sunday.kind, DayKind::Off);
        assert!(s.now(date(2025, 6, 8), at(3, 0), NO_HOLIDAYS).current.is_none());
    }

    #[test]
    fn holidays_use_the_holiday_plan_unless_not_observed() {
        let mut s = worker();
        let christmas = date(2025, 12, 25); // a Thursday
        let is_holiday = |d: NaiveDate| d == christmas;
        let now = s.now(christmas, at(12, 30), &is_holiday);
        assert!(now.holiday);
        assert_eq!(now.current.unwrap().activity, "family lunch");

        s.observes_holidays = false;
        let now = s.now(christmas, at(12, 30), &is_holiday);
        assert!(!now.holiday);
        assert_eq!(now.current.unwrap().activity, "work");
    }

    #[test]
    fn malformed_blocks_are_ignored_and_emptiness_is_detected() {
        let mut s = Schedule::default();
        assert!(s.is_empty());
        s.days[0].blocks = vec![block("8", "late", "work", None), block("09:00", "10:00", " ", None)];
        assert!(!s.is_empty());
        assert!(s.now(date(2025, 6, 2), at(9, 30), NO_HOLIDAYS).current.is_none());
    }

    #[test]
    fn short_day_lists_and_old_json_still_load() {
        let s: Schedule = serde_json::from_str(r#"{"days":[{"kind":"school"}]}"#).unwrap();
        assert!(s.observes_holidays);
        assert_eq!(s.now(date(2025, 6, 2), at(9, 0), NO_HOLIDAYS).kind, DayKind::School);
        assert_eq!(s.now(date(2025, 6, 3), at(9, 0), NO_HOLIDAYS).kind, DayKind::Off);
    }
}
