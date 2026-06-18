//! This module contains generated bindings, and code to make bindings more ergonomic
//!
// use std::collections::HashMap;
// use std::error::Error;
// use std::str::FromStr;

use anyhow::{bail, Context as _};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use mysql_async::{Row, Value as MySqlValue};
use num_traits::Float;
use serde_json;

// Bindgen happens here
wit_bindgen_wrpc::generate!({
    with: {
        "wasmcloud:mysql/query@1.0.0" : generate,
        "wasmcloud:mysql/prepared@1.0.0" : generate,
        "wasmcloud:mysql/types@1.0.0" : generate,
    },
});

// Start bindgen-generated type imports
pub(crate) use exports::wasmcloud::mysql::prepared;
pub(crate) use exports::wasmcloud::mysql::query;

pub(crate) use query::{QueryError, ResultRow};

pub(crate) use prepared::{
    PreparedStatementExecError, PreparedStatementToken, StatementPrepareError,
};

use crate::bindings::wasmcloud::mysql::types::{
    Date, Datetime, MysqlValue, ResultRowEntry, Time, Timestamp, Year,
};
// End of bindgen-generated type imports

/// Build an `f64` from a mantissa, exponent and sign
fn f64_from_components(mantissa: u64, exponent: i16, sign: i8) -> f64 {
    let sign_f = sign as f64;
    let mantissa_f = mantissa as f64;
    let exponent_f = 2f64.powf(exponent as f64);
    sign_f * mantissa_f * exponent_f
}

/// Build an `f64` from a simple tuple of mantissa, exponent and sign
fn f64_from_tuple(t: &(u64, i16, i8)) -> f64 {
    f64_from_components(t.0, t.1, t.2)
}

/// Convert MySQL date to chrono NaiveDate
impl TryFrom<&Date> for NaiveDate {
    type Error = anyhow::Error;

    fn try_from(d: &Date) -> anyhow::Result<NaiveDate> {
        NaiveDate::from_ymd_opt(d.year as i32, d.month, d.day).with_context(|| {
            format!(
                "failed to build date from ymd ({}/{}/{})",
                d.year, d.month, d.day
            )
        })
    }
}

impl From<NaiveDate> for Date {
    fn from(nd: NaiveDate) -> Date {
        Date {
            year: nd.year() as u32,
            month: nd.month(),
            day: nd.day(),
        }
    }
}

/// Convert MySQL time to chrono NaiveTime
impl TryFrom<&Time> for NaiveTime {
    type Error = anyhow::Error;

    fn try_from(t: &Time) -> anyhow::Result<NaiveTime> {
        // Handle negative time (MySQL supports negative TIME for intervals)
        if t.negative {
            bail!("Negative time intervals cannot be converted to NaiveTime");
        }

        NaiveTime::from_hms_micro_opt(t.hour, t.minute, t.second, t.microsecond).with_context(
            || {
                format!(
                    "failed to convert time [{}h {}m {}s {}micro]",
                    t.hour, t.minute, t.second, t.microsecond
                )
            },
        )
    }
}

impl From<NaiveTime> for Time {
    fn from(nt: NaiveTime) -> Time {
        Time {
            negative: false,
            hour: nt.hour(),
            minute: nt.minute(),
            second: nt.second(),
            microsecond: nt.nanosecond() / 1_000,
        }
    }
}

/// Convert MySQL datetime to chrono NaiveDateTime
impl TryFrom<&Datetime> for NaiveDateTime {
    type Error = anyhow::Error;

    fn try_from(dt: &Datetime) -> anyhow::Result<NaiveDateTime> {
        let date: NaiveDate = (&dt.date).try_into()?;
        let time: NaiveTime = (&dt.time).try_into()?;
        Ok(NaiveDateTime::new(date, time))
    }
}

impl From<NaiveDateTime> for Datetime {
    fn from(ndt: NaiveDateTime) -> Datetime {
        Datetime {
            date: ndt.date().into(),
            time: ndt.time().into(),
        }
    }
}

/// Convert MySQL timestamp to chrono NaiveDateTime
impl TryFrom<&Timestamp> for NaiveDateTime {
    type Error = anyhow::Error;

    fn try_from(ts: &Timestamp) -> anyhow::Result<NaiveDateTime> {
        (&ts.datetime).try_into()
    }
}

impl From<NaiveDateTime> for Timestamp {
    fn from(ndt: NaiveDateTime) -> Timestamp {
        Timestamp {
            datetime: ndt.into(),
        }
    }
}

/// Error conversion functions
pub(crate) fn mysql_error_to_query_error(err: mysql_async::Error) -> QueryError {
    match err {
        mysql_async::Error::Driver(driver_err) => {
            QueryError::InvalidParams(format!("Driver error: {}", driver_err))
        }
        mysql_async::Error::Server(server_err) => {
            if server_err.message.contains("syntax error")
                || server_err.message.contains("SQL syntax")
            {
                QueryError::InvalidQuery(format!("SQL syntax error: {}", server_err.message))
            } else {
                QueryError::InvalidParams(format!("Server error: {}", server_err.message))
            }
        }
        mysql_async::Error::Io(io_err) => QueryError::Unexpected(format!("IO error: {}", io_err)),
        mysql_async::Error::Url(url_err) => {
            QueryError::Unexpected(format!("Connection URL error: {}", url_err))
        }
        other => QueryError::Unexpected(format!("MySQL error: {}", other)),
    }
}

pub(crate) fn mysql_error_to_prepared_error(err: mysql_async::Error) -> PreparedStatementExecError {
    match err {
        mysql_async::Error::Server(server_err)
            if server_err.message.contains("Unknown prepared statement") =>
        {
            PreparedStatementExecError::UnknownPreparedQuery
        }
        other => PreparedStatementExecError::QueryError(mysql_error_to_query_error(other)),
    }
}

pub(crate) fn mysql_error_to_statement_prepare_error(
    err: mysql_async::Error,
) -> StatementPrepareError {
    StatementPrepareError::Unexpected(format!("Failed to prepare statement: {}", err))
}

/// Build a `ResultRow` from a MySQL [`Row`]
pub(crate) fn into_result_row(row: Row) -> anyhow::Result<ResultRow> {
    let mut result_entries: ResultRow = Vec::new();
    let columns = row.columns_ref();

    for (i, column) in columns.iter().enumerate() {
        let mysql_val: MySqlValue = row.get(i).unwrap_or(MySqlValue::NULL);
        let wit_val: MysqlValue = mysql_value_to_wit_value(mysql_val, column)?;

        result_entries.push(ResultRowEntry {
            column_name: column.name_str().to_string(),
            value: wit_val,
        });
    }

    Ok(result_entries)
}

/// Enhanced conversion from MySQL driver value to WIT value using column metadata
fn mysql_value_to_wit_value(
    mysql_val: MySqlValue,
    column: &mysql_async::Column,
) -> anyhow::Result<MysqlValue> {
    use mysql_async::consts::ColumnType;

    match mysql_val {
        MySqlValue::NULL => Ok(MysqlValue::Null),
        MySqlValue::Bytes(bytes) => {
            // Use column type information for accurate conversion
            match column.column_type() {
                ColumnType::MYSQL_TYPE_JSON => {
                    let json_str = String::from_utf8(bytes)
                        .with_context(|| "Failed to convert JSON bytes to UTF-8")?;
                    Ok(MysqlValue::Json(json_str))
                }
                ColumnType::MYSQL_TYPE_ENUM => {
                    let enum_str = String::from_utf8(bytes)
                        .with_context(|| "Failed to convert ENUM bytes to UTF-8")?;
                    Ok(MysqlValue::Enum(enum_str))
                }
                ColumnType::MYSQL_TYPE_SET => {
                    let set_str = String::from_utf8(bytes)
                        .with_context(|| "Failed to convert SET bytes to UTF-8")?;
                    let set_values: Vec<String> = if set_str.is_empty() {
                        Vec::new()
                    } else {
                        set_str.split(',').map(|s| s.to_string()).collect()
                    };
                    Ok(MysqlValue::Set(set_values))
                }
                ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => {
                    let decimal_str = String::from_utf8(bytes)
                        .with_context(|| "Failed to convert DECIMAL bytes to UTF-8")?;
                    Ok(MysqlValue::Decimal(decimal_str))
                }
                ColumnType::MYSQL_TYPE_BIT => {
                    let bit_length = column.column_length() as u8;
                    Ok(MysqlValue::Bit((bit_length, bytes.into())))
                }
                ColumnType::MYSQL_TYPE_GEOMETRY => Ok(MysqlValue::Geometry(bytes.into())),
                ColumnType::MYSQL_TYPE_TINY_BLOB => Ok(MysqlValue::Tinyblob(bytes.into())),
                ColumnType::MYSQL_TYPE_MEDIUM_BLOB => Ok(MysqlValue::Mediumblob(bytes.into())),
                ColumnType::MYSQL_TYPE_LONG_BLOB => Ok(MysqlValue::Longblob(bytes.into())),
                ColumnType::MYSQL_TYPE_BLOB => Ok(MysqlValue::Blob(bytes.into())),
                ColumnType::MYSQL_TYPE_VAR_STRING | ColumnType::MYSQL_TYPE_STRING => {
                    let length = column.column_length();
                    match String::from_utf8(bytes.clone()) {
                        Ok(s) => {
                            if column
                                .flags()
                                .contains(mysql_async::consts::ColumnFlags::BINARY_FLAG)
                            {
                                // BINARY or VARBINARY
                                if column.column_type() == ColumnType::MYSQL_TYPE_STRING {
                                    Ok(MysqlValue::Binary((length, bytes.into())))
                                } else {
                                    Ok(MysqlValue::Varbinary((length, bytes.into())))
                                }
                            } else {
                                // CHAR or VARCHAR
                                if column.column_type() == ColumnType::MYSQL_TYPE_STRING {
                                    Ok(MysqlValue::Char((length, s)))
                                } else {
                                    Ok(MysqlValue::Varchar((length, s)))
                                }
                            }
                        }
                        Err(_) => {
                            // Not valid UTF-8, treat as binary
                            if column.column_type() == ColumnType::MYSQL_TYPE_STRING {
                                Ok(MysqlValue::Binary((length, bytes.into())))
                            } else {
                                Ok(MysqlValue::Varbinary((length, bytes.into())))
                            }
                        }
                    }
                }
                _ => {
                    // Default handling for other byte types
                    match String::from_utf8(bytes.clone()) {
                        Ok(s) => match column.column_type() {
                            ColumnType::MYSQL_TYPE_TINY => Ok(MysqlValue::Tinytext(s)),
                            ColumnType::MYSQL_TYPE_LONG => Ok(MysqlValue::Longtext(s)),
                            _ => Ok(MysqlValue::Text(s)),
                        },
                        Err(_) => Ok(MysqlValue::Blob(bytes.into())),
                    }
                }
            }
        }
        other_val => Ok(other_val.into()),
    }
}

/// Convert MySQL driver value to our WIT MysqlValue (fallback without column info)
impl From<MySqlValue> for MysqlValue {
    fn from(mysql_val: MySqlValue) -> MysqlValue {
        match mysql_val {
            MySqlValue::NULL => MysqlValue::Null,

            MySqlValue::Bytes(bytes) => match String::from_utf8(bytes.clone()) {
                Ok(s) => {
                    if s.trim_start().starts_with(['{', '[']) {
                        MysqlValue::Json(s)
                    } else if s.chars().all(|c| {
                        c.is_ascii_digit()
                            || c == '.'
                            || c == '-'
                            || c == '+'
                            || c == 'e'
                            || c == 'E'
                    }) && s.contains('.')
                    {
                        MysqlValue::Decimal(s)
                    } else {
                        MysqlValue::Text(s)
                    }
                }
                Err(_) => MysqlValue::Blob(bytes.into()),
            },

            MySqlValue::Int(n) => {
                if n >= i8::MIN as i64 && n <= i8::MAX as i64 {
                    MysqlValue::Tinyint(n as i8)
                } else if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
                    MysqlValue::Smallint(n as i16)
                } else if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                    MysqlValue::Int(n as i32)
                } else {
                    MysqlValue::Bigint(n)
                }
            }

            MySqlValue::UInt(n) => {
                if n <= u8::MAX as u64 {
                    MysqlValue::TinyintUnsigned(n as u8)
                } else if n <= u16::MAX as u64 {
                    MysqlValue::SmallintUnsigned(n as u16)
                } else if n <= u32::MAX as u64 {
                    MysqlValue::IntUnsigned(n as u32)
                } else {
                    MysqlValue::BigintUnsigned(n)
                }
            }

            MySqlValue::Float(f) => MysqlValue::Float(f.integer_decode()),
            MySqlValue::Double(f) => MysqlValue::Double(f.integer_decode()),

            MySqlValue::Date(year, month, day, hour, minute, second, micro) => {
                if hour == 0 && minute == 0 && second == 0 && micro == 0 {
                    MysqlValue::Date(Date {
                        year: year as u32,
                        month: month as u32,
                        day: day as u32,
                    })
                } else {
                    MysqlValue::Datetime(Datetime {
                        date: Date {
                            year: year as u32,
                            month: month as u32,
                            day: day as u32,
                        },
                        time: Time {
                            negative: false,
                            hour: hour as u32,
                            minute: minute as u32,
                            second: second as u32,
                            microsecond: micro,
                        },
                    })
                }
            }

            MySqlValue::Time(negative, days, hours, minutes, seconds, microseconds) => {
                let total_hours = (days * 24) + hours as u32;
                MysqlValue::Time(Time {
                    negative,
                    hour: total_hours,
                    minute: minutes as u32,
                    second: seconds as u32,
                    microsecond: microseconds,
                })
            }
        }
    }
}

/// Convert our WIT MysqlValue to MySQL driver value
impl From<MysqlValue> for MySqlValue {
    fn from(wit_val: MysqlValue) -> MySqlValue {
        match wit_val {
            MysqlValue::Null => MySqlValue::NULL,

            MysqlValue::Tinyint(n) => MySqlValue::Int(n as i64),
            MysqlValue::TinyintUnsigned(n) => MySqlValue::UInt(n as u64),
            MysqlValue::Smallint(n) => MySqlValue::Int(n as i64),
            MysqlValue::SmallintUnsigned(n) => MySqlValue::UInt(n as u64),
            MysqlValue::Mediumint(n) => MySqlValue::Int(n as i64),
            MysqlValue::MediumintUnsigned(n) => MySqlValue::UInt(n as u64),
            MysqlValue::Int(n) | MysqlValue::Integer(n) => MySqlValue::Int(n as i64),
            MysqlValue::IntUnsigned(n) | MysqlValue::IntegerUnsigned(n) => {
                MySqlValue::UInt(n as u64)
            }
            MysqlValue::Bigint(n) => MySqlValue::Int(n),
            MysqlValue::BigintUnsigned(n) => MySqlValue::UInt(n),

            MysqlValue::Decimal(d) | MysqlValue::Numeric(d) => {
                MySqlValue::Bytes(d.as_bytes().to_vec())
            }

            MysqlValue::Float(f) => MySqlValue::Float(f64_from_tuple(&f) as f32),
            MysqlValue::Double(f) | MysqlValue::Real(f) => MySqlValue::Double(f64_from_tuple(&f)),

            MysqlValue::Bit((_, bytes)) => MySqlValue::Bytes(bytes.to_vec()),

            MysqlValue::Bool(b) | MysqlValue::Boolean(b) => MySqlValue::Int(if b { 1 } else { 0 }),

            MysqlValue::Date(d) => {
                MySqlValue::Date(d.year as u16, d.month as u8, d.day as u8, 0, 0, 0, 0)
            }
            MysqlValue::Time(t) => {
                let days = t.hour / 24;
                let hours = (t.hour % 24) as u8;
                MySqlValue::Time(
                    t.negative,
                    days,
                    hours,
                    t.minute as u8,
                    t.second as u8,
                    t.microsecond,
                )
            }
            MysqlValue::Datetime(dt) => MySqlValue::Date(
                dt.date.year as u16,
                dt.date.month as u8,
                dt.date.day as u8,
                dt.time.hour as u8,
                dt.time.minute as u8,
                dt.time.second as u8,
                dt.time.microsecond,
            ),
            MysqlValue::Timestamp(ts) => {
                // Same as datetime - because MySQL handles UTC conversion internally
                MySqlValue::Date(
                    ts.datetime.date.year as u16,
                    ts.datetime.date.month as u8,
                    ts.datetime.date.day as u8,
                    ts.datetime.time.hour as u8,
                    ts.datetime.time.minute as u8,
                    ts.datetime.time.second as u8,
                    ts.datetime.time.microsecond,
                )
            }
            MysqlValue::Year(year) => match year {
                Year::YearTwo(y) => MySqlValue::UInt(y as u64),
                Year::YearFour(y) => MySqlValue::UInt(y as u64),
            },

            MysqlValue::Char((_, s)) | MysqlValue::Varchar((_, s)) => {
                MySqlValue::Bytes(s.as_bytes().to_vec())
            }
            MysqlValue::Text(s)
            | MysqlValue::Tinytext(s)
            | MysqlValue::Mediumtext(s)
            | MysqlValue::Longtext(s) => MySqlValue::Bytes(s.as_bytes().to_vec()),

            MysqlValue::Binary((_, bytes)) | MysqlValue::Varbinary((_, bytes)) => {
                MySqlValue::Bytes(bytes.to_vec())
            }
            MysqlValue::Blob(bytes)
            | MysqlValue::Tinyblob(bytes)
            | MysqlValue::Mediumblob(bytes)
            | MysqlValue::Longblob(bytes) => MySqlValue::Bytes(bytes.to_vec()),

            MysqlValue::Json(s) => MySqlValue::Bytes(s.as_bytes().to_vec()),

            MysqlValue::Enum(s) => MySqlValue::Bytes(s.as_bytes().to_vec()),
            MysqlValue::Set(set) => MySqlValue::Bytes(set.join(",").as_bytes().to_vec()),

            MysqlValue::Geometry(wkb)
            | MysqlValue::PointGeom(wkb)
            | MysqlValue::Linestring(wkb)
            | MysqlValue::Polygon(wkb)
            | MysqlValue::Multipoint(wkb)
            | MysqlValue::Multilinestring(wkb)
            | MysqlValue::Multipolygon(wkb)
            | MysqlValue::Geometrycollection(wkb) => MySqlValue::Bytes(wkb.to_vec()),
        }
    }
}

/// Validate that a MysqlValue is compatible with the expected MySQL column type
pub(crate) fn validate_mysql_value_for_column_type(
    value: &MysqlValue,
    expected_type: mysql_async::consts::ColumnType,
) -> anyhow::Result<()> {
    use mysql_async::consts::ColumnType;

    match (value, expected_type) {
        (MysqlValue::Null, _) => Ok(()), // NULL is valid for all column typs

        (MysqlValue::Tinyint(_) | MysqlValue::TinyintUnsigned(_), ColumnType::MYSQL_TYPE_TINY) => {
            Ok(())
        }
        (
            MysqlValue::Smallint(_) | MysqlValue::SmallintUnsigned(_),
            ColumnType::MYSQL_TYPE_SHORT,
        ) => Ok(()),
        (
            MysqlValue::Mediumint(_) | MysqlValue::MediumintUnsigned(_),
            ColumnType::MYSQL_TYPE_INT24,
        ) => Ok(()),
        (
            MysqlValue::Int(_)
            | MysqlValue::Integer(_)
            | MysqlValue::IntUnsigned(_)
            | MysqlValue::IntegerUnsigned(_),
            ColumnType::MYSQL_TYPE_LONG,
        ) => Ok(()),
        (
            MysqlValue::Bigint(_) | MysqlValue::BigintUnsigned(_),
            ColumnType::MYSQL_TYPE_LONGLONG,
        ) => Ok(()),

        (MysqlValue::Float(_), ColumnType::MYSQL_TYPE_FLOAT) => Ok(()),
        (MysqlValue::Double(_) | MysqlValue::Real(_), ColumnType::MYSQL_TYPE_DOUBLE) => Ok(()),

        (
            MysqlValue::Decimal(_) | MysqlValue::Numeric(_),
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL,
        ) => Ok(()),

        (MysqlValue::Date(_), ColumnType::MYSQL_TYPE_DATE) => Ok(()),
        (MysqlValue::Time(_), ColumnType::MYSQL_TYPE_TIME) => Ok(()),
        (MysqlValue::Datetime(_), ColumnType::MYSQL_TYPE_DATETIME) => Ok(()),
        (MysqlValue::Timestamp(_), ColumnType::MYSQL_TYPE_TIMESTAMP) => Ok(()),
        (MysqlValue::Year(_), ColumnType::MYSQL_TYPE_YEAR) => Ok(()),

        (MysqlValue::Char(..), ColumnType::MYSQL_TYPE_STRING) => Ok(()),
        (MysqlValue::Varchar(..), ColumnType::MYSQL_TYPE_VAR_STRING) => Ok(()),
        (MysqlValue::Text(_), ColumnType::MYSQL_TYPE_BLOB) => Ok(()),
        (MysqlValue::Tinytext(_), ColumnType::MYSQL_TYPE_TINY_BLOB) => Ok(()),
        (MysqlValue::Mediumtext(_), ColumnType::MYSQL_TYPE_MEDIUM_BLOB) => Ok(()),
        (MysqlValue::Longtext(_), ColumnType::MYSQL_TYPE_LONG_BLOB) => Ok(()),

        (
            MysqlValue::Binary(..) | MysqlValue::Varbinary(..),
            ColumnType::MYSQL_TYPE_STRING | ColumnType::MYSQL_TYPE_VAR_STRING,
        ) => Ok(()),
        (MysqlValue::Blob(_), ColumnType::MYSQL_TYPE_BLOB) => Ok(()),
        (MysqlValue::Tinyblob(_), ColumnType::MYSQL_TYPE_TINY_BLOB) => Ok(()),
        (MysqlValue::Mediumblob(_), ColumnType::MYSQL_TYPE_MEDIUM_BLOB) => Ok(()),
        (MysqlValue::Longblob(_), ColumnType::MYSQL_TYPE_LONG_BLOB) => Ok(()),

        (MysqlValue::Bool(_) | MysqlValue::Boolean(_), ColumnType::MYSQL_TYPE_TINY) => Ok(()), // BOOL is TINYINT(1) for reference
        (MysqlValue::Bit(..), ColumnType::MYSQL_TYPE_BIT) => Ok(()),
        (MysqlValue::Enum(_), ColumnType::MYSQL_TYPE_ENUM) => Ok(()),
        (MysqlValue::Set(_), ColumnType::MYSQL_TYPE_SET) => Ok(()),
        (MysqlValue::Json(_), ColumnType::MYSQL_TYPE_JSON) => Ok(()),

        (
            MysqlValue::Geometry(_)
            | MysqlValue::PointGeom(_)
            | MysqlValue::Linestring(_)
            | MysqlValue::Polygon(_)
            | MysqlValue::Multipoint(_)
            | MysqlValue::Multilinestring(_)
            | MysqlValue::Multipolygon(_)
            | MysqlValue::Geometrycollection(_),
            ColumnType::MYSQL_TYPE_GEOMETRY,
        ) => Ok(()),

        (value, expected) => {
            bail!(
                "Type mismatch: cannot use {:?} for column type {:?}",
                std::mem::discriminant(value),
                expected
            )
        }
    }
}

/// Convert a SET string back to a vector of strings
pub(crate) fn parse_mysql_set(set_str: &str) -> Vec<String> {
    if set_str.is_empty() {
        Vec::new()
    } else {
        set_str.split(',').map(|s| s.to_string()).collect()
    }
}

/// Join a SET vector into a MySQL SET string
pub(crate) fn format_mysql_set(set_values: &[String]) -> String {
    set_values.join(",")
}

/// Handle MySQL YEAR type conversion
pub(crate) fn mysql_year_to_u16(year: &Year) -> u16 {
    match year {
        Year::YearTwo(y) => {
            // MySQL 2-digit year conversion rules are as fllows:
            // 00-69 -> 2000-2069
            // 70-99 -> 1970-1999
            if *y <= 69 {
                2000 + *y as u16
            } else {
                1900 + *y as u16
            }
        }
        Year::YearFour(y) => *y as u16,
    }
}

/// Convert u16 year back to MySQL Year type
pub(crate) fn u16_to_mysql_year(year: u16) -> Year {
    if year >= 2000 && year <= 2069 {
        Year::YearTwo((year - 2000) as u8)
    } else if year >= 1970 && year <= 1999 {
        Year::YearTwo((year - 1900) as u8)
    } else {
        Year::YearFour(year as u32)
    }
}

/// JSON validation helper -- can never be too safe
pub(crate) fn validate_json_string(json_str: &str) -> anyhow::Result<()> {
    serde_json::from_str::<serde_json::Value>(json_str)
        .with_context(|| format!("Invalid JSON string: {}", json_str))?;
    Ok(())
}

/// Spatial data helpers -- might need em later on
pub(crate) fn is_valid_wkb(wkb: &[u8]) -> bool {
    // Basic WKB validation - must have at least 5 bytes (byte order + geometry type)
    wkb.len() >= 5
}
