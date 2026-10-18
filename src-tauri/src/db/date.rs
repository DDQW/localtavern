//! Calendar-date columns (birthdays) are stored as `YYYY-MM-DD` text; see `rows::opt_date` for reading.

use chrono::NaiveDate;

pub fn format_opt(date: Option<NaiveDate>) -> Option<String> {
    date.map(|d| d.format("%Y-%m-%d").to_string())
}
