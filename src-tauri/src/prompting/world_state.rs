use chrono::{Datelike, NaiveDate, Weekday};

use super::instructions::{self, Instructions};
use crate::holidays::{DatedHoliday, SchoolBreak};
use crate::models::{DayKind, DayPlan, Meter, Schedule, WorldState};

/// Days ahead of a birthday at which the model is told it is coming up.
const BIRTHDAY_NOTICE_DAYS: i64 = 7;
/// Days ahead of a holiday at which the model is told it is coming up, and how many to list.
const HOLIDAY_NOTICE_DAYS: i64 = 7;
const MAX_UPCOMING_HOLIDAYS: usize = 3;

/// What the world-state block knows besides the clock itself.
#[derive(Debug, Default)]
pub struct WorldFacts<'a> {
    /// `(name, birth date)` of the people in the scene.
    pub birthdays: Vec<(&'a str, NaiveDate)>,
    /// Where the story is set, e.g. "Germany (DE-BY)".
    pub place: Option<String>,
    /// Holidays from the day before the story date to a week after (national + the story's own).
    pub holidays: Vec<DatedHoliday>,
    /// School holidays where the story is set that overlap the next two weeks, earliest first.
    pub school: Vec<SchoolBreak>,
    /// `(name, meter)` of the characters whose arousal the story tracks (empty when it doesn't).
    pub meters: Vec<(&'a str, Meter)>,
    /// `(name, routine)` of the characters that have one.
    pub schedules: Vec<(&'a str, &'a Schedule)>,
}

/// Turns a conversation's world state into the text injected via `PromptBlockType::WorldState`:
/// the clock and weather, then where the story is set and what kind of day it is (weekday,
/// weekend, public holiday, upcoming holidays), what each character's routine has them doing, and
/// birthdays with ages as of the in-story date.
pub fn format(ins: &Instructions, state: &WorldState, facts: &WorldFacts) -> String {
    let weather = match state.weather_notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(notes) => format!("{} ({notes})", state.weather.name()),
        None => state.weather.name().to_string(),
    };
    // en-US long format, e.g. "Saturday, June 14, 2025 3:45 PM"
    let when = state.fictional_date_time.format("%A, %B %-d, %Y %-I:%M %p");
    let mut lines = vec![ins.render(instructions::WORLD_NOW, &[("when", &when.to_string()), ("weather", &weather)])];
    let today = state.fictional_date_time.date();

    if let Some(place) = facts.place.as_deref() {
        lines.push(ins.render(instructions::WORLD_PLACE, &[("place", place)]));
    }
    let day_off_on = |date: NaiveDate| facts.holidays.iter().any(|h| h.date == date && h.day_off);
    let todays: Vec<&DatedHoliday> = facts.holidays.iter().filter(|h| h.date == today).collect();
    if !todays.is_empty() {
        let names = todays.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join(" / ");
        let key = if day_off_on(today) { instructions::WORLD_HOLIDAY } else { instructions::WORLD_OBSERVANCE };
        lines.push(ins.render(key, &[("names", &names)]));
    } else if matches!(today.weekday(), Weekday::Sat | Weekday::Sun) {
        lines.push(ins.render(instructions::WORLD_WEEKEND, &[]));
    } else if facts.place.is_some() || !facts.schedules.is_empty() {
        lines.push(ins.render(instructions::WORLD_WEEKDAY, &[]));
    }
    let upcoming: Vec<String> = facts
        .holidays
        .iter()
        .filter(|h| (1..=HOLIDAY_NOTICE_DAYS).contains(&(h.date - today).num_days()))
        .take(MAX_UPCOMING_HOLIDAYS)
        .map(|h| format!("{} ({})", h.name, h.date.format("%A, %B %-d")))
        .collect();
    if !upcoming.is_empty() {
        lines.push(ins.render(instructions::WORLD_UPCOMING, &[("list", &upcoming.join("; "))]));
    }

    let school_out = facts.school.iter().find(|b| b.covers(today));
    if let Some(b) = school_out {
        let end = b.end.format("%A, %B %-d").to_string();
        lines.push(ins.render(instructions::WORLD_SCHOOL_OUT, &[("name", &b.name), ("end", &end)]));
    } else if let Some(b) = facts.school.iter().find(|b| b.start > today) {
        let (start, end) = (b.start.format("%A, %B %-d").to_string(), b.end.format("%B %-d").to_string());
        lines.push(ins.render(instructions::WORLD_SCHOOL_SOON, &[("name", &b.name), ("start", &start), ("end", &end)]));
    }

    let routines: Vec<String> = facts
        .schedules
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|&(name, schedule)| routine_line(name.trim(), schedule, state, &day_off_on, school_out.is_some()))
        .collect();
    if !routines.is_empty() {
        lines.push(ins.render(instructions::WORLD_ROUTINES, &[("list", &routines.join(" "))]));
    }

    let meters: Vec<String> = facts
        .meters
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|(name, m)| format!("{} - arousal {}/100, climax {}/100 (climaxed {}x so far).", name.trim(), m.arousal, m.climax, m.orgasms))
        .collect();
    if !meters.is_empty() {
        lines.push(ins.render(instructions::WORLD_METERS, &[("list", &meters.join(" "))]));
    }

    let birthdays: Vec<String> = facts
        .birthdays
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|&(name, born)| birthday_line(name.trim(), born, today))
        .collect();
    if !birthdays.is_empty() {
        lines.push(ins.render(instructions::WORLD_BIRTHDAYS, &[("list", &birthdays.join(" "))]));
    }
    lines.join("\n")
}

/// "Mira - workday; now: work (bakery) until 16:00; next: gym at 18:00."
fn routine_line(name: &str, schedule: &Schedule, state: &WorldState, is_holiday: &dyn Fn(NaiveDate) -> bool, school_out: bool) -> String {
    let at = state.fictional_date_time;
    // In school holidays a school-day plan is a free day.
    let freed;
    let mut schedule = schedule;
    let mut label = None;
    if school_out && schedule.days.iter().any(|d| d.kind == DayKind::School) {
        let mut copy = schedule.clone();
        for plan in copy.days.iter_mut().filter(|d| d.kind == DayKind::School) {
            *plan = DayPlan::default();
        }
        label = schedule.plan_for(at.date(), is_holiday(at.date())).0.filter(|p| p.kind == DayKind::School).map(|_| "school holidays");
        freed = copy;
        schedule = &freed;
    }
    let now = schedule.now(at.date(), at.time(), is_holiday);
    let day = if now.holiday { "holiday" } else { label.unwrap_or(now.kind.label()) };
    let mut line = format!("{name} - {day}; now: ");
    match now.current {
        Some(block) => line.push_str(&format!("{} until {}", block.describe(), block.end.trim())),
        None => line.push_str("free time"),
    }
    if let Some(next) = now.next {
        line.push_str(&format!("; next: {} at {}", next.describe(), next.start.trim()));
    }
    line.push('.');
    line
}

fn birthday_line(name: &str, born: NaiveDate, today: NaiveDate) -> String {
    let age = age_on(born, today);
    let mut line = match age {
        Some(age) => format!("{name} was born {} ({age} years old).", born.format("%B %-d, %Y")),
        // Born after the story's date (e.g. a story set in another era): only the day is meaningful.
        None => format!("{name}'s birthday is {}.", born.format("%B %-d")),
    };
    let mut next = birthday_in(born, today.year());
    if next < today {
        next = birthday_in(born, today.year() + 1);
    }
    // On the day the age has just ticked over; before it, they turn one older.
    let turns = |age: i32| format!(" ({name} turns {age})");
    match (next - today).num_days() {
        0 => line.push_str(&format!(" Today is {name}'s birthday{}.", age.map(turns).unwrap_or_default())),
        1 => line.push_str(&format!(" {name}'s birthday is tomorrow{}.", age.map(|a| turns(a + 1)).unwrap_or_default())),
        days if days <= BIRTHDAY_NOTICE_DAYS => line.push_str(&format!(
            " {name}'s birthday is in {days} days{}.",
            age.map(|a| turns(a + 1)).unwrap_or_default()
        )),
        _ => {}
    }
    line
}

/// The day the birthday is celebrated in `year`; February 29 falls back to February 28.
fn birthday_in(born: NaiveDate, year: i32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, born.month(), born.day())
        .or_else(|| NaiveDate::from_ymd_opt(year, 2, 28))
        .expect("February 28 exists every year")
}

/// Completed years on `today`, or `None` when `today` is before the birth date.
pub fn age_on(born: NaiveDate, today: NaiveDate) -> Option<i32> {
    if today < born {
        return None;
    }
    let had_birthday = birthday_in(born, today.year()) <= today;
    Some(today.year() - born.year() - if had_birthday { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WeatherCondition;
    use chrono::NaiveDate;

    fn format_plain(state: &WorldState) -> String {
        format(instructions::defaults(), state, &WorldFacts::default())
    }

    fn born<'a>(list: &[(&'a str, NaiveDate)]) -> WorldFacts<'a> {
        WorldFacts { birthdays: list.to_vec(), ..Default::default() }
    }

    fn on(y: i32, m: u32, d: u32) -> WorldState {
        WorldState {
            fictional_date_time: NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(9, 0, 0).unwrap(),
            ..WorldState::create_default()
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn state(weather: WeatherCondition, notes: Option<&str>) -> WorldState {
        WorldState {
            fictional_date_time: NaiveDate::from_ymd_opt(2025, 6, 14).unwrap().and_hms_opt(15, 45, 0).unwrap(),
            weather,
            weather_notes: notes.map(String::from),
            ..WorldState::create_default()
        }
    }

    #[test]
    fn includes_date_time_and_weather() {
        let text = format_plain(&state(WeatherCondition::Rain, None));
        assert!(text.contains("Saturday, June 14, 2025 3:45 PM"), "{text}");
        assert!(text.contains("Rain"));
    }

    #[test]
    fn puts_weather_notes_in_parens() {
        assert!(format_plain(&state(WeatherCondition::Rain, Some("light drizzle"))).contains("Rain (light drizzle)"));
    }

    #[test]
    fn omits_parens_when_notes_are_blank() {
        assert!(!format_plain(&state(WeatherCondition::Clear, Some("  "))).contains('('));
        assert!(!format_plain(&state(WeatherCondition::Clear, None)).contains('('));
    }

    #[test]
    fn midnight_and_noon_use_twelve_hour_clock() {
        let mut s = state(WeatherCondition::Clear, None);
        s.fictional_date_time = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(0, 5, 0).unwrap();
        assert!(format_plain(&s).contains("12:05 AM"));
        s.fictional_date_time = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert!(format_plain(&s).contains("12:00 PM"));
    }

    #[test]
    fn no_birthdays_means_no_birthday_line() {
        assert!(!format(instructions::defaults(), &on(2025, 6, 1), &WorldFacts::default()).contains("Birthdays"));
    }

    #[test]
    fn gives_age_as_of_the_story_date() {
        let text = format(instructions::defaults(), &on(2025, 6, 1), &born(&[("Mira", date(2001, 3, 3)), ("Alex", date(2000, 12, 24))]));
        assert!(text.ends_with("
Birthdays: Mira was born March 3, 2001 (24 years old). Alex was born December 24, 2000 (24 years old)."), "{text}");
    }

    #[test]
    fn announces_the_birthday_itself() {
        let text = format(instructions::defaults(), &on(2025, 3, 3), &born(&[("Mira", date(2001, 3, 3))]));
        assert!(text.contains("Mira was born March 3, 2001 (24 years old). Today is Mira's birthday (Mira turns 24)."), "{text}");
    }

    #[test]
    fn announces_tomorrow_and_the_coming_week_but_not_later() {
        assert!(format(instructions::defaults(), &on(2025, 3, 2), &born(&[("Mira", date(2001, 3, 3))])).contains("Mira's birthday is tomorrow (Mira turns 24)."));
        assert!(format(instructions::defaults(), &on(2025, 2, 28), &born(&[("Mira", date(2001, 3, 3))])).contains("Mira's birthday is in 3 days (Mira turns 24)."));
        assert!(!format(instructions::defaults(), &on(2025, 2, 20), &born(&[("Mira", date(2001, 3, 3))])).contains("birthday is"));
    }

    #[test]
    fn upcoming_birthday_across_new_year() {
        let text = format(instructions::defaults(), &on(2025, 12, 30), &born(&[("Noel", date(1990, 1, 2))]));
        assert!(text.contains("Noel was born January 2, 1990 (35 years old). Noel's birthday is in 3 days (Noel turns 36)."), "{text}");
    }

    #[test]
    fn leap_day_birthdays_are_celebrated_on_february_28() {
        let text = format(instructions::defaults(), &on(2025, 2, 28), &born(&[("Leo", date(2004, 2, 29))]));
        assert!(text.contains("(21 years old). Today is Leo's birthday (Leo turns 21)."), "{text}");
        assert!(format(instructions::defaults(), &on(2024, 2, 29), &born(&[("Leo", date(2004, 2, 29))])).contains("Today is Leo's birthday (Leo turns 20)."));
    }

    #[test]
    fn birth_after_the_story_date_gives_only_the_day() {
        let text = format(instructions::defaults(), &on(1250, 3, 1), &born(&[("Mira", date(2001, 3, 3))]));
        assert!(text.contains("Mira's birthday is March 3. Mira's birthday is in 2 days."), "{text}");
        assert!(!text.contains("years old"), "{text}");
    }

    fn worker() -> Schedule {
        use crate::models::{DayKind, DayPlan, ScheduleBlock};
        let block = |start: &str, end: &str, activity: &str| ScheduleBlock {
            start: start.into(),
            end: end.into(),
            activity: activity.into(),
            place: None,
        };
        let mut s = Schedule::default();
        for d in 0..5 {
            s.days[d] = DayPlan { kind: DayKind::Work, blocks: vec![block("08:00", "16:00", "work"), block("18:00", "19:00", "gym")] };
        }
        s.holiday = DayPlan { kind: DayKind::Off, blocks: vec![block("09:00", "11:00", "brunch")] };
        s
    }

    fn holiday(d: NaiveDate, name: &str, day_off: bool) -> DatedHoliday {
        DatedHoliday { date: d, name: name.into(), day_off }
    }

    #[test]
    fn a_plain_clock_says_nothing_about_the_day() {
        let text = format_plain(&on(2025, 6, 2));
        assert!(!text.contains('\n'), "{text}");
    }

    #[test]
    fn names_the_setting_weekday_and_routines() {
        let schedule = worker();
        let facts = WorldFacts {
            place: Some("Germany (DE-BY)".into()),
            schedules: vec![("Mira", &schedule)],
            ..Default::default()
        };
        let text = format(instructions::defaults(), &on(2025, 6, 2), &facts); // Monday 09:00
        assert!(text.contains("\nThe story is set in Germany (DE-BY).\nIt is a regular weekday."), "{text}");
        assert!(text.contains("Mira - workday; now: work until 16:00; next: gym at 18:00."), "{text}");
    }

    #[test]
    fn weekends_and_free_time() {
        let schedule = worker();
        let facts = WorldFacts { schedules: vec![("Mira", &schedule)], ..Default::default() };
        let text = format(instructions::defaults(), &on(2025, 6, 7), &facts); // Saturday
        assert!(text.contains("It is the weekend."), "{text}");
        assert!(text.contains("Mira - day off; now: free time."), "{text}");
    }

    #[test]
    fn holidays_today_and_upcoming_switch_the_routine() {
        let schedule = worker();
        let facts = WorldFacts {
            holidays: vec![
                holiday(date(2025, 12, 25), "Christmas Day", true),
                holiday(date(2025, 12, 26), "St. Stephen's Day", true),
                holiday(date(2025, 12, 31), "New Year's Eve", false),
                holiday(date(2026, 1, 9), "Too far", true),
            ],
            schedules: vec![("Mira", &schedule)],
            ..Default::default()
        };
        let text = format(instructions::defaults(), &on(2025, 12, 25), &facts); // Thursday 09:00
        assert!(text.contains("Today is a public holiday: Christmas Day. Most people have the day off."), "{text}");
        assert!(text.contains("Upcoming holidays: St. Stephen's Day (Friday, December 26); New Year's Eve (Wednesday, December 31)."), "{text}");
        assert!(!text.contains("Too far"), "{text}");
        assert!(text.contains("Mira - holiday; now: brunch until 11:00."), "{text}");

        let eve = format(instructions::defaults(), &on(2025, 12, 31), &facts);
        assert!(eve.contains("Today is New Year's Eve (not a day off)."), "{eve}");
        assert!(eve.contains("Mira - workday; now: work"), "{eve}");
    }
}
