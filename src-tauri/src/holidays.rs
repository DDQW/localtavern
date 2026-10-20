//! Public holidays for the country a roleplay is set in, from the free Nager.Date service
//! (`date.nager.at`), and school holidays (Schulferien) from OpenHolidays (`openholidaysapi.org`), cached in AppSettings so each country/year is fetched once. Lookups never
//! block or fail a reply: a miss just means no holidays until the background fetch lands.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::CustomHoliday;
use crate::repo::app_settings;

/// One public holiday as Nager.Date describes it (unknown fields are ignored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holiday {
    pub date: NaiveDate,
    #[serde(default)]
    pub local_name: String,
    pub name: String,
    /// Nationwide; otherwise only in `counties`.
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub counties: Option<Vec<String>>,
    /// "Public", "Bank", "School", "Authorities", "Optional", "Observance".
    #[serde(default)]
    pub types: Vec<String>,
}

impl Holiday {
    /// Whether it is observed where the story is set. Regional holidays need the region to match.
    pub fn applies_in(&self, region: Option<&str>) -> bool {
        self.global
            || region.is_some_and(|r| self.counties.as_ref().is_some_and(|c| c.iter().any(|x| x.eq_ignore_ascii_case(r))))
    }

    /// Most people have the day off (as opposed to bank-only or observance days).
    pub fn is_day_off(&self) -> bool {
        self.types.is_empty() || self.types.iter().any(|t| t == "Public")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Country {
    pub country_code: String,
    pub name: String,
}

/// A holiday on a given story date, from the national data or the story's own list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatedHoliday {
    pub date: NaiveDate,
    pub name: String,
    pub day_off: bool,
}

/// A stretch of school holidays (Schulferien), e.g. "Autumn Holidays" 5-17 October in Hesse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolBreak {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub name: String,
    #[serde(default)]
    pub nationwide: bool,
    /// Region codes it applies to (e.g. "DE-HE") when not nationwide.
    #[serde(default)]
    pub regions: Vec<String>,
}

impl SchoolBreak {
    /// Whether it applies where the story is set. Dates differ by region, so one needs a match.
    pub fn applies_in(&self, region: Option<&str>) -> bool {
        self.nationwide || region.is_some_and(|r| self.regions.iter().any(|x| x.eq_ignore_ascii_case(r)))
    }

    pub fn covers(&self, date: NaiveDate) -> bool {
        (self.start..=self.end).contains(&date)
    }
}

#[async_trait]
pub trait HolidaySource: Send + Sync {
    async fn countries(&self) -> std::result::Result<Vec<Country>, String>;
    /// An empty list for a year or country the source doesn't cover.
    async fn public_holidays(&self, country: &str, year: i32) -> std::result::Result<Vec<Holiday>, String>;
    /// School holidays overlapping the year; empty for a country the source doesn't cover.
    async fn school_holidays(&self, _country: &str, _year: i32) -> std::result::Result<Vec<SchoolBreak>, String> {
        Ok(Vec::new())
    }
}

pub struct NagerHolidays {
    client: reqwest::Client,
    base_url: String,
    school_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenEntry {
    start_date: NaiveDate,
    end_date: NaiveDate,
    #[serde(default)]
    name: Vec<OpenText>,
    #[serde(default)]
    nationwide: bool,
    #[serde(default)]
    subdivisions: Option<Vec<OpenSubdivision>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct OpenText {
    text: String,
}

#[derive(Deserialize)]
struct OpenSubdivision {
    code: String,
}

impl NagerHolidays {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_base_url(client, "https://date.nager.at").with_school_url("https://openholidaysapi.org")
    }

    pub fn with_base_url(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        Self { client, school_url: base_url.clone(), base_url }
    }

    /// Where school holidays come from (the same host as the public holidays unless set).
    pub fn with_school_url(mut self, url: impl Into<String>) -> Self {
        self.school_url = url.into();
        self
    }

    async fn get_json<T: serde::de::DeserializeOwned + Default>(&self, path: &str) -> std::result::Result<T, String> {
        let response = self
            .client
            .get(format!("{}{path}", self.base_url))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Couldn't reach the holiday service: {e}"))?;
        let status = response.status();
        // 204 / 404: a year or country the service has no data for.
        if status == reqwest::StatusCode::NO_CONTENT || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(T::default());
        }
        if !status.is_success() {
            return Err(format!("The holiday service answered {status}."));
        }
        let body = response.bytes().await.map_err(|e| format!("Couldn't read the holiday data: {e}"))?;
        serde_json::from_slice(&body).map_err(|e| format!("Unexpected holiday data: {e}"))
    }
}

#[async_trait]
impl HolidaySource for NagerHolidays {
    async fn countries(&self) -> std::result::Result<Vec<Country>, String> {
        self.get_json("/api/v3/AvailableCountries").await
    }

    async fn public_holidays(&self, country: &str, year: i32) -> std::result::Result<Vec<Holiday>, String> {
        if !(1..=9999).contains(&year) {
            return Ok(Vec::new());
        }
        self.get_json(&format!("/api/v3/PublicHolidays/{year}/{}", country.to_ascii_uppercase())).await
    }

    async fn school_holidays(&self, country: &str, year: i32) -> std::result::Result<Vec<SchoolBreak>, String> {
        if !(1..=9999).contains(&year) {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .get(format!(
                "{}/SchoolHolidays?countryIsoCode={}&validFrom={year}-01-01&validTo={year}-12-31&languageIsoCode=EN",
                self.school_url,
                country.to_ascii_uppercase()
            ))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Couldn't reach the school holiday service: {e}"))?;
        let status = response.status();
        // 204 / 400 / 404: a country (or year) the service has no school data for.
        if matches!(status.as_u16(), 204 | 400 | 404) {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            return Err(format!("The school holiday service answered {status}."));
        }
        let body = response.bytes().await.map_err(|e| format!("Couldn't read the school holiday data: {e}"))?;
        let entries: Vec<OpenEntry> = serde_json::from_slice(&body).map_err(|e| format!("Unexpected school holiday data: {e}"))?;
        Ok(entries
            .into_iter()
            // Exceptions are for a few places inside a region (e.g. North Sea islands).
            .filter(|e| !e.tags.iter().flatten().any(|t| t.eq_ignore_ascii_case("Exception")))
            .map(|e| SchoolBreak {
                start: e.start_date,
                end: e.end_date,
                name: e.name.into_iter().next().map(|n| n.text).unwrap_or_else(|| "School holidays".into()),
                nationwide: e.nationwide,
                regions: e.subdivisions.into_iter().flatten().map(|s| s.code).collect(),
            })
            .collect())
    }
}

// ---- cache ----

/// A cached lookup. A failed one is kept for the day it failed (so a reply doesn't retry on every
/// message), then retried; a successful one is kept for good.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached<T> {
    fetched_on: NaiveDate,
    failed: bool,
    data: T,
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn holidays_key(country: &str, year: i32) -> String {
    format!("holidays.{}.{year}", country.to_ascii_uppercase())
}

fn school_key(country: &str, year: i32) -> String {
    format!("schoolholidays.{}.{year}", country.to_ascii_uppercase())
}

const COUNTRIES_KEY: &str = "holidays.countries";

fn read<T: serde::de::DeserializeOwned>(conn: &Connection, key: &str, today: NaiveDate) -> Result<Option<T>> {
    let cached = app_settings::get(conn, key)?.and_then(|json| serde_json::from_str::<Cached<T>>(&json).ok());
    Ok(cached.filter(|c| !c.failed || c.fetched_on == today).map(|c| c.data))
}

fn write<T: Serialize>(conn: &Connection, key: &str, data: &T, failed: bool, today: NaiveDate) -> Result<()> {
    app_settings::set(conn, key, &serde_json::to_string(&Cached { fetched_on: today, failed, data })?)
}

/// The cached holidays for a country and year; `None` when they still need fetching.
pub fn cached_holidays(conn: &Connection, country: &str, year: i32) -> Result<Option<Vec<Holiday>>> {
    read(conn, &holidays_key(country, year), today())
}

pub fn store_holidays(conn: &Connection, country: &str, year: i32, fetched: &std::result::Result<Vec<Holiday>, String>) -> Result<()> {
    match fetched {
        Ok(list) => write(conn, &holidays_key(country, year), list, false, today()),
        Err(_) => write(conn, &holidays_key(country, year), &Vec::<Holiday>::new(), true, today()),
    }
}

/// The cached school holidays for a country and year; `None` when they still need fetching.
pub fn cached_school_breaks(conn: &Connection, country: &str, year: i32) -> Result<Option<Vec<SchoolBreak>>> {
    read(conn, &school_key(country, year), today())
}

pub fn store_school_breaks(conn: &Connection, country: &str, year: i32, fetched: &std::result::Result<Vec<SchoolBreak>, String>) -> Result<()> {
    match fetched {
        Ok(list) => write(conn, &school_key(country, year), list, false, today()),
        Err(_) => write(conn, &school_key(country, year), &Vec::<SchoolBreak>::new(), true, today()),
    }
}

/// The school holidays in force where the story is set that overlap `from..=to` (from whatever is
/// cached), earliest first.
pub fn school_breaks_between(conn: &Connection, country: Option<&str>, region: Option<&str>, from: NaiveDate, to: NaiveDate) -> Result<Vec<SchoolBreak>> {
    use chrono::Datelike;
    let mut found = Vec::new();
    if let Some(country) = country.filter(|c| !c.trim().is_empty()) {
        for year in from.year()..=to.year() {
            for b in cached_school_breaks(conn, country, year)?.unwrap_or_default() {
                if b.start <= to && b.end >= from && b.applies_in(region) && !found.contains(&b) {
                    found.push(b);
                }
            }
        }
    }
    found.sort_by_key(|b| b.start);
    Ok(found)
}

pub fn cached_countries(conn: &Connection) -> Result<Option<Vec<Country>>> {
    read(conn, COUNTRIES_KEY, today())
}

pub fn store_countries(conn: &Connection, countries: &[Country]) -> Result<()> {
    write(conn, COUNTRIES_KEY, &countries, false, today())
}

/// The English name of a country code, from the cached list (the code itself when unknown).
pub fn country_name(conn: &Connection, code: &str) -> Result<String> {
    Ok(cached_countries(conn)?
        .and_then(|list| list.into_iter().find(|c| c.country_code.eq_ignore_ascii_case(code)))
        .map(|c| c.name)
        .unwrap_or_else(|| code.to_ascii_uppercase()))
}

/// Every holiday from `from` to `to` (inclusive) where the story is set: national ones that apply
/// in the region (from whatever is cached) plus the story's own.
pub fn holidays_between(
    conn: &Connection,
    country: Option<&str>,
    region: Option<&str>,
    custom: &[CustomHoliday],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<DatedHoliday>> {
    use chrono::Datelike;
    let mut found = Vec::new();
    if let Some(country) = country.filter(|c| !c.trim().is_empty()) {
        for year in from.year()..=to.year() {
            for h in cached_holidays(conn, country, year)?.unwrap_or_default() {
                if (from..=to).contains(&h.date) && h.applies_in(region) {
                    let day_off = h.is_day_off();
                    found.push(DatedHoliday { date: h.date, name: h.name, day_off });
                }
            }
        }
    }
    for day in from.iter_days().take_while(|d| *d <= to) {
        for c in custom.iter().filter(|c| c.falls_on(day) && !c.name.trim().is_empty()) {
            found.push(DatedHoliday { date: day, name: c.name.trim().to_string(), day_off: true });
        }
    }
    found.sort_by_key(|h| h.date);
    found.dedup_by(|a, b| a.date == b.date && a.name.eq_ignore_ascii_case(&b.name));
    Ok(found)
}

/// The region codes the year's holidays mention (e.g. "DE-BY"), for the region picker.
pub fn regions(holidays: &[Holiday]) -> Vec<String> {
    let mut all: Vec<String> = holidays.iter().flat_map(|h| h.counties.clone().unwrap_or_default()).collect();
    all.sort();
    all.dedup();
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn holiday(d: NaiveDate, name: &str, global: bool, counties: &[&str], types: &[&str]) -> Holiday {
        Holiday {
            date: d,
            local_name: name.into(),
            name: name.into(),
            global,
            counties: (!counties.is_empty()).then(|| counties.iter().map(|c| c.to_string()).collect()),
            types: types.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn regional_holidays_need_the_region() {
        let epiphany = holiday(date(2025, 1, 6), "Epiphany", false, &["DE-BW", "DE-BY"], &["Public"]);
        assert!(!epiphany.applies_in(None));
        assert!(!epiphany.applies_in(Some("DE-BE")));
        assert!(epiphany.applies_in(Some("de-by")));
        assert!(holiday(date(2025, 1, 1), "New Year", true, &[], &["Public"]).applies_in(None));
    }

    #[test]
    fn only_public_holidays_are_days_off() {
        assert!(holiday(date(2025, 1, 1), "x", true, &[], &["Public"]).is_day_off());
        assert!(!holiday(date(2025, 1, 1), "x", true, &[], &["Bank", "Observance"]).is_day_off());
    }

    #[test]
    fn between_merges_cached_national_and_custom_holidays_across_new_year() {
        let conn = crate::db::open_in_memory().unwrap();
        let national = vec![
            holiday(date(2025, 12, 25), "Christmas Day", true, &[], &["Public"]),
            holiday(date(2025, 12, 26), "St. Stephen's Day", true, &[], &["Public"]),
            holiday(date(2025, 11, 1), "All Saints' Day", false, &["DE-BY"], &["Public"]),
        ];
        store_holidays(&conn, "DE", 2025, &Ok(national)).unwrap();
        store_holidays(&conn, "DE", 2026, &Ok(vec![holiday(date(2026, 1, 1), "New Year's Day", true, &[], &["Public"])])).unwrap();
        let custom = [
            CustomHoliday { name: "Founders' Night".into(), month: 12, day: 30, year: None },
            CustomHoliday { name: "Once".into(), month: 12, day: 31, year: Some(2024) },
        ];

        let found = holidays_between(&conn, Some("de"), None, &custom, date(2025, 12, 24), date(2026, 1, 2)).unwrap();
        let names: Vec<_> = found.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["Christmas Day", "St. Stephen's Day", "Founders' Night", "New Year's Day"]);
        assert!(holidays_between(&conn, Some("DE"), Some("DE-BY"), &[], date(2025, 11, 1), date(2025, 11, 1)).unwrap()[0].day_off);
    }

    #[test]
    fn failed_lookups_are_retried_on_a_later_day_and_successes_are_kept() {
        let conn = crate::db::open_in_memory().unwrap();
        assert_eq!(cached_holidays(&conn, "DE", 2025).unwrap(), None);
        store_holidays(&conn, "DE", 2025, &Err("offline".into())).unwrap();
        assert_eq!(cached_holidays(&conn, "DE", 2025).unwrap(), Some(vec![]), "not retried the same day");

        let yesterday = today().pred_opt().unwrap();
        write(&conn, &holidays_key("DE", 2025), &Vec::<Holiday>::new(), true, yesterday).unwrap();
        assert_eq!(cached_holidays(&conn, "DE", 2025).unwrap(), None, "retried the next day");
        write(&conn, &holidays_key("DE", 2025), &Vec::<Holiday>::new(), false, yesterday).unwrap();
        assert_eq!(cached_holidays(&conn, "DE", 2025).unwrap(), Some(vec![]), "an empty success is kept");
    }

    #[test]
    fn school_breaks_need_the_region_and_survive_new_year() {
        let conn = crate::db::open_in_memory().unwrap();
        let winter = SchoolBreak { start: date(2025, 12, 22), end: date(2026, 1, 5), name: "Winter Holidays".into(), nationwide: false, regions: vec!["DE-HE".into()] };
        let other = SchoolBreak { start: date(2025, 12, 22), end: date(2026, 1, 2), name: "Winter Holidays".into(), nationwide: false, regions: vec!["DE-BY".into()] };
        for year in [2025, 2026] {
            store_school_breaks(&conn, "DE", year, &Ok(vec![winter.clone(), other.clone()])).unwrap();
        }
        let found = school_breaks_between(&conn, Some("de"), Some("de-he"), date(2025, 12, 30), date(2026, 1, 3)).unwrap();
        assert_eq!(found, vec![winter.clone()], "one entry, only for the region");
        assert!(school_breaks_between(&conn, Some("DE"), None, date(2025, 12, 30), date(2026, 1, 3)).unwrap().is_empty());
        assert!(winter.covers(date(2026, 1, 5)) && !winter.covers(date(2026, 1, 6)));
    }

    #[tokio::test]
    async fn school_holidays_are_read_from_openholidays_and_exceptions_are_skipped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/SchoolHolidays"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"startDate":"2026-10-05","endDate":"2026-10-17","type":"School","name":[{"language":"EN","text":"Autumn Holidays"}],
                     "nationwide":false,"subdivisions":[{"code":"DE-HE","shortName":"HE"}]},
                    {"startDate":"2026-10-05","endDate":"2026-10-24","type":"School","name":[{"language":"EN","text":"Autumn Holidays"}],
                     "nationwide":false,"subdivisions":[{"code":"DE-SH","shortName":"SH"}],"tags":["Exception"]}]"#,
            ))
            .mount(&server)
            .await;
        let source = NagerHolidays::with_base_url(reqwest::Client::new(), server.uri());
        let list = source.school_holidays("de", 2026).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].end, list[0].regions.clone()), ("Autumn Holidays", date(2026, 10, 17), vec!["DE-HE".to_string()]));
        assert_eq!(source.school_holidays("de", 0).await.unwrap(), vec![]);
    }

    #[test]
    fn country_names_come_from_the_cached_list() {
        let conn = crate::db::open_in_memory().unwrap();
        assert_eq!(country_name(&conn, "de").unwrap(), "DE");
        store_countries(&conn, &[Country { country_code: "DE".into(), name: "Germany".into() }]).unwrap();
        assert_eq!(country_name(&conn, "de").unwrap(), "Germany");
    }

    #[test]
    fn regions_are_collected_from_the_holidays() {
        let list = [
            holiday(date(2025, 1, 6), "a", false, &["DE-BY", "DE-BW"], &[]),
            holiday(date(2025, 3, 8), "b", false, &["DE-BE", "DE-BY"], &[]),
        ];
        assert_eq!(regions(&list), ["DE-BE", "DE-BW", "DE-BY"]);
    }

    #[tokio::test]
    async fn nager_client_parses_holidays_and_treats_missing_data_as_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/PublicHolidays/2025/DE"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"date":"2025-01-01","localName":"Neujahr","name":"New Year's Day","countryCode":"DE","fixed":false,
                    "global":true,"counties":null,"launchYear":null,"types":["Public"]}]"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/PublicHolidays/1250/DE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/AvailableCountries"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"[{"countryCode":"DE","name":"Germany"}]"#))
            .mount(&server)
            .await;

        let nager = NagerHolidays::with_base_url(reqwest::Client::new(), server.uri());
        let list = nager.public_holidays("de", 2025).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].local_name, "Neujahr");
        assert!(list[0].global && list[0].is_day_off());
        assert!(nager.public_holidays("DE", 1250).await.unwrap().is_empty());
        assert_eq!(nager.countries().await.unwrap()[0].name, "Germany");
    }

    /// Hits the real service; run by hand with `cargo test --lib live_nager -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_nager_lookup() {
        let nager = NagerHolidays::new(crate::providers::http::build_client());
        let list = nager.public_holidays("DE", 2025).await.unwrap();
        assert!(list.iter().any(|h| h.name == "Christmas Day"), "{list:?}");
        assert!(nager.countries().await.unwrap().len() > 50);
    }
}
