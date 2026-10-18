use chrono::{Datelike, NaiveDate};

use crate::models::WorldState;

/// Days ahead of a birthday at which the model is told it is coming up.
const BIRTHDAY_NOTICE_DAYS: i64 = 7;

/// Turns a conversation's world state into the text injected via `PromptBlockType::WorldState`,
/// followed by the birthdays of the people in the scene (`(name, birth date)`), with ages as of the
/// in-story date and a heads-up on or shortly before each birthday.
pub fn format(state: &WorldState, birthdays: &[(&str, NaiveDate)]) -> String {
    let weather = match state.weather_notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(notes) => format!("{} ({notes})", state.weather.name()),
        None => state.weather.name().to_string(),
    };
    // en-US long format, e.g. "Saturday, June 14, 2025 3:45 PM"
    let when = state.fictional_date_time.format("%A, %B %-d, %Y %-I:%M %p");
    let clock = format!(
        "Current in-story date/time: {when}. Current weather: {weather}. \
Keep the narration consistent with this date/time and weather unless the story explicitly changes it."
    );
    let today = state.fictional_date_time.date();
    let lines: Vec<String> = birthdays
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|&(name, born)| birthday_line(name.trim(), born, today))
        .collect();
    if lines.is_empty() {
        clock
    } else {
        format!("{clock}\nBirthdays: {}", lines.join(" "))
    }
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
fn age_on(born: NaiveDate, today: NaiveDate) -> Option<i32> {
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
        format(state, &[])
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
        assert!(!format(&on(2025, 6, 1), &[]).contains("Birthdays"));
    }

    #[test]
    fn gives_age_as_of_the_story_date() {
        let text = format(&on(2025, 6, 1), &[("Mira", date(2001, 3, 3)), ("Alex", date(2000, 12, 24))]);
        assert!(text.ends_with("
Birthdays: Mira was born March 3, 2001 (24 years old). Alex was born December 24, 2000 (24 years old)."), "{text}");
    }

    #[test]
    fn announces_the_birthday_itself() {
        let text = format(&on(2025, 3, 3), &[("Mira", date(2001, 3, 3))]);
        assert!(text.contains("Mira was born March 3, 2001 (24 years old). Today is Mira's birthday (Mira turns 24)."), "{text}");
    }

    #[test]
    fn announces_tomorrow_and_the_coming_week_but_not_later() {
        assert!(format(&on(2025, 3, 2), &[("Mira", date(2001, 3, 3))]).contains("Mira's birthday is tomorrow (Mira turns 24)."));
        assert!(format(&on(2025, 2, 28), &[("Mira", date(2001, 3, 3))]).contains("Mira's birthday is in 3 days (Mira turns 24)."));
        assert!(!format(&on(2025, 2, 20), &[("Mira", date(2001, 3, 3))]).contains("birthday is"));
    }

    #[test]
    fn upcoming_birthday_across_new_year() {
        let text = format(&on(2025, 12, 30), &[("Noel", date(1990, 1, 2))]);
        assert!(text.contains("Noel was born January 2, 1990 (35 years old). Noel's birthday is in 3 days (Noel turns 36)."), "{text}");
    }

    #[test]
    fn leap_day_birthdays_are_celebrated_on_february_28() {
        let text = format(&on(2025, 2, 28), &[("Leo", date(2004, 2, 29))]);
        assert!(text.contains("(21 years old). Today is Leo's birthday (Leo turns 21)."), "{text}");
        assert!(format(&on(2024, 2, 29), &[("Leo", date(2004, 2, 29))]).contains("Today is Leo's birthday (Leo turns 20)."));
    }

    #[test]
    fn birth_after_the_story_date_gives_only_the_day() {
        let text = format(&on(1250, 3, 1), &[("Mira", date(2001, 3, 3))]);
        assert!(text.contains("Mira's birthday is March 3. Mira's birthday is in 2 days."), "{text}");
        assert!(!text.contains("years old"), "{text}");
    }
}
