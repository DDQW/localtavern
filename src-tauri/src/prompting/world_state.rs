use crate::models::WorldState;

/// Turns a conversation's world state into the text injected via `PromptBlockType::WorldState`.
pub fn format(state: &WorldState) -> String {
    let weather = match state.weather_notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(notes) => format!("{} ({notes})", state.weather.name()),
        None => state.weather.name().to_string(),
    };
    // en-US long format, e.g. "Saturday, June 14, 2025 3:45 PM"
    let when = state.fictional_date_time.format("%A, %B %-d, %Y %-I:%M %p");
    format!(
        "Current in-story date/time: {when}. Current weather: {weather}. \
Keep the narration consistent with this date/time and weather unless the story explicitly changes it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WeatherCondition;
    use chrono::NaiveDate;

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
        let text = format(&state(WeatherCondition::Rain, None));
        assert!(text.contains("Saturday, June 14, 2025 3:45 PM"), "{text}");
        assert!(text.contains("Rain"));
    }

    #[test]
    fn puts_weather_notes_in_parens() {
        assert!(format(&state(WeatherCondition::Rain, Some("light drizzle"))).contains("Rain (light drizzle)"));
    }

    #[test]
    fn omits_parens_when_notes_are_blank() {
        assert!(!format(&state(WeatherCondition::Clear, Some("  "))).contains('('));
        assert!(!format(&state(WeatherCondition::Clear, None)).contains('('));
    }

    #[test]
    fn midnight_and_noon_use_twelve_hour_clock() {
        let mut s = state(WeatherCondition::Clear, None);
        s.fictional_date_time = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(0, 5, 0).unwrap();
        assert!(format(&s).contains("12:05 AM"));
        s.fictional_date_time = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert!(format(&s).contains("12:00 PM"));
    }
}
