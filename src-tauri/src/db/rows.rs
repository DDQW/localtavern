//! Typed column readers for row-mapping closures. They return `rusqlite::Result` so they compose
//! with `query_map`; a value that fails to parse becomes a `FromSqlConversionFailure` naming the column.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{types::Type, Row};
use serde::de::DeserializeOwned;
use uuid::Uuid;

fn bad(row: &Row, col: &str, err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> rusqlite::Error {
    let idx = row.as_ref().column_index(col).unwrap_or(0);
    rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, err.into())
}

pub fn uuid(row: &Row, col: &str) -> rusqlite::Result<Uuid> {
    let s: String = row.get(col)?;
    Uuid::parse_str(&s).map_err(|e| bad(row, col, e))
}

pub fn opt_uuid(row: &Row, col: &str) -> rusqlite::Result<Option<Uuid>> {
    let s: Option<String> = row.get(col)?;
    s.map(|s| Uuid::parse_str(&s).map_err(|e| bad(row, col, e))).transpose()
}

pub fn datetime(row: &Row, col: &str) -> rusqlite::Result<DateTime<Utc>> {
    let s: String = row.get(col)?;
    super::time::parse(&s).map_err(|e| bad(row, col, e))
}

/// A `YYYY-MM-DD` date column.
pub fn opt_date(row: &Row, col: &str) -> rusqlite::Result<Option<NaiveDate>> {
    let s: Option<String> = row.get(col)?;
    s.map(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|e| bad(row, col, e))).transpose()
}

pub fn blob<T: DeserializeOwned>(row: &Row, col: &str) -> rusqlite::Result<T> {
    let s: String = row.get(col)?;
    super::json::from_blob(&s).map_err(|e| bad(row, col, e.to_string()))
}

pub fn opt_blob<T: DeserializeOwned>(row: &Row, col: &str) -> rusqlite::Result<Option<T>> {
    let s: Option<String> = row.get(col)?;
    s.map(|s| super::json::from_blob(&s).map_err(|e| bad(row, col, e.to_string()))).transpose()
}

pub fn int_enum<T>(row: &Row, col: &str, from: fn(i32) -> Option<T>) -> rusqlite::Result<T> {
    let v: i32 = row.get(col)?;
    from(v).ok_or_else(|| bad(row, col, format!("unknown enum value {v}")))
}

pub fn opt_int_enum<T>(row: &Row, col: &str, from: fn(i32) -> Option<T>) -> rusqlite::Result<Option<T>> {
    let v: Option<i32> = row.get(col)?;
    v.map(|v| from(v).ok_or_else(|| bad(row, col, format!("unknown enum value {v}")))).transpose()
}
