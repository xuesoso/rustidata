//! Cell values and the column type system.
//!
//! Mirrors visidata's model: stores yield *raw* values (often byte slices);
//! a column's `VType` is an interpretation applied at read time.

use std::fmt;
use std::sync::Arc;

use compact_str::{CompactString, ToCompactString};

/// A fully-realized cell value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Microseconds since the Unix epoch.
    Date(i64),
    Str(CompactString),
    /// A cell-level error (failed parse, expression error). Rendered with a note.
    Error(Arc<str>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub fn from_str_auto(s: &str) -> Value {
        if s.is_empty() {
            return Value::Null;
        }
        Value::Str(s.into())
    }

    /// Best-effort numeric view, used by aggregators and plotting.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(*b as i64 as f64),
            Value::Date(us) => Some(*us as f64),
            Value::Str(s) => parse_float_str(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            Value::Bool(b) => Some(*b as i64),
            Value::Date(us) => Some(*us),
            Value::Str(s) => parse_int_str(s),
            _ => None,
        }
    }

    /// The untyped display string (no fmtstr applied).
    pub fn to_display(&self) -> CompactString {
        match self {
            Value::Null => CompactString::default(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Int(i) => i.to_compact_string(),
            Value::Float(f) => format_float(*f),
            Value::Date(us) => format_date(*us, "%Y-%m-%d %H:%M:%S"),
            Value::Str(s) => s.clone(),
            Value::Error(e) => CompactString::from(&**e),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_display())
    }
}

/// Column types, matching visidata's set (`~ # % @ $` notes etc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VType {
    #[default]
    Any,
    Str,
    Int,
    Float,
    Date,
    Currency,
    /// Length of the value (visidata's `vlen`).
    Len,
    /// Float with SI suffix parsing/formatting (1.5M, 2k...).
    FloatSI,
}

impl VType {
    /// The one-character type note shown in headers (visidata parity).
    pub fn note(&self) -> &'static str {
        match self {
            VType::Any => "",
            VType::Str => "~",
            VType::Int => "#",
            VType::Float => "%",
            VType::Date => "@",
            VType::Currency => "$",
            VType::Len => "#",
            VType::FloatSI => "%",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            VType::Any => "any",
            VType::Str => "str",
            VType::Int => "int",
            VType::Float => "float",
            VType::Date => "date",
            VType::Currency => "currency",
            VType::Len => "len",
            VType::FloatSI => "floatsi",
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            VType::Int | VType::Float | VType::Currency | VType::Len | VType::FloatSI
        )
    }

    /// Coerce a raw string into this type. Empty input is Null; failures are Error.
    pub fn parse(&self, s: &str) -> Value {
        if s.is_empty() {
            return Value::Null;
        }
        match self {
            VType::Any => Value::Str(s.into()),
            VType::Str => Value::Str(s.into()),
            VType::Int => match parse_int_str(s) {
                Some(i) => Value::Int(i),
                None => Value::Error(err_parse(s, "int")),
            },
            VType::Float => match parse_float_str(s) {
                Some(f) => Value::Float(f),
                None => Value::Error(err_parse(s, "float")),
            },
            VType::Date => match parse_date(s) {
                Some(us) => Value::Date(us),
                None => Value::Error(err_parse(s, "date")),
            },
            VType::Currency => match parse_currency(s) {
                Some(f) => Value::Float(f),
                None => Value::Error(err_parse(s, "currency")),
            },
            VType::Len => Value::Int(s.chars().count() as i64),
            VType::FloatSI => match parse_floatsi(s) {
                Some(f) => Value::Float(f),
                None => Value::Error(err_parse(s, "floatsi")),
            },
        }
    }

    /// Coerce an already-realized value into this type.
    pub fn coerce(&self, v: Value) -> Value {
        match (self, &v) {
            (VType::Any, _) => v,
            (_, Value::Null) | (_, Value::Error(_)) => v,
            (VType::Str, _) => Value::Str(v.to_display()),
            (VType::Int, Value::Int(_)) => v,
            (VType::Int, _) => match v.as_i64() {
                Some(i) => Value::Int(i),
                None => Value::Error(err_parse(&v.to_display(), "int")),
            },
            (VType::Float | VType::Currency | VType::FloatSI, Value::Float(_)) => v,
            (VType::Float | VType::Currency | VType::FloatSI, _) => match v.as_f64() {
                Some(f) => Value::Float(f),
                None => Value::Error(err_parse(&v.to_display(), "float")),
            },
            (VType::Date, Value::Date(_)) => v,
            (VType::Date, Value::Str(s)) => match parse_date(s) {
                Some(us) => Value::Date(us),
                None => Value::Error(err_parse(s, "date")),
            },
            (VType::Date, _) => Value::Error(err_parse(&v.to_display(), "date")),
            (VType::Len, Value::Str(s)) => Value::Int(s.chars().count() as i64),
            (VType::Len, _) => Value::Int(v.to_display().chars().count() as i64),
        }
    }

    /// Format a typed value for display, honoring an optional fmtstr.
    pub fn format(&self, v: &Value, fmtstr: Option<&str>) -> CompactString {
        match v {
            Value::Null => return CompactString::default(),
            Value::Error(e) => return CompactString::from(&**e),
            _ => {}
        }
        match self {
            VType::Date => {
                if let Value::Date(us) = v {
                    return format_date(*us, fmtstr.unwrap_or("%Y-%m-%d %H:%M:%S"));
                }
                v.to_display()
            }
            VType::Currency => match v.as_f64() {
                Some(f) => format_thousands(f, 2).into(),
                None => v.to_display(),
            },
            VType::FloatSI => match v.as_f64() {
                Some(f) => format_si(f).into(),
                None => v.to_display(),
            },
            VType::Float => match (v.as_f64(), fmtstr) {
                (Some(f), Some(spec)) => format_float_spec(f, spec),
                (Some(f), None) => format_float(f),
                _ => v.to_display(),
            },
            _ => v.to_display(),
        }
    }

    pub fn all() -> &'static [VType] {
        &[
            VType::Any,
            VType::Str,
            VType::Int,
            VType::Float,
            VType::Date,
            VType::Currency,
            VType::Len,
            VType::FloatSI,
        ]
    }
}

fn err_parse(s: &str, ty: &str) -> Arc<str> {
    Arc::from(format!("'{}' not {}", s.chars().take(20).collect::<String>(), ty).as_str())
}

/// Tolerant int parse: trims whitespace, allows thousands separators and leading +.
pub fn parse_int_str(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let cleaned: String = t.chars().filter(|c| *c != ',' && *c != '_').collect();
    cleaned.parse::<i64>().ok().or_else(|| {
        // "3.0" should parse as int 3 when exact
        let f = cleaned.parse::<f64>().ok()?;
        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
            Some(f as i64)
        } else {
            None
        }
    })
}

/// Tolerant float parse: trims, allows thousands separators and percent suffix.
pub fn parse_float_str(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (t, scale) = match t.strip_suffix('%') {
        Some(rest) => (rest, 0.01),
        None => (t, 1.0),
    };
    let cleaned: String = t.chars().filter(|c| *c != ',' && *c != '_').collect();
    cleaned.parse::<f64>().ok().map(|f| f * scale)
}

fn parse_currency(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+' || *c == 'e' || *c == 'E')
        .collect();
    cleaned.parse::<f64>().ok()
}

fn parse_floatsi(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (num, suffix) = t.split_at(t.len() - t.chars().last()?.len_utf8());
    let mult = match suffix {
        "k" | "K" => 1e3,
        "M" => 1e6,
        "G" | "B" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "m" => 1e-3,
        "u" | "µ" => 1e-6,
        "n" => 1e-9,
        _ => return parse_float_str(t),
    };
    parse_float_str(num).map(|f| f * mult)
}

fn format_si(f: f64) -> String {
    let a = f.abs();
    let (scaled, suffix) = if a >= 1e12 {
        (f / 1e12, "T")
    } else if a >= 1e9 {
        (f / 1e9, "G")
    } else if a >= 1e6 {
        (f / 1e6, "M")
    } else if a >= 1e3 {
        (f / 1e3, "k")
    } else {
        (f, "")
    };
    format!("{:.2}{}", scaled, suffix)
}

fn format_thousands(f: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, f.abs());
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s.as_str(), ""));
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    let digits: Vec<char> = int_part.chars().collect();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    if !frac_part.is_empty() {
        out.push('.');
        out.push_str(frac_part);
    }
    out
}

fn format_float(f: f64) -> CompactString {
    if f == f.trunc() && f.abs() < 1e15 {
        // show "3.0" not "3" to keep the float-ness visible
        format!("{:.1}", f).into()
    } else {
        let mut s = format!("{:.6}", f);
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.push('0');
        }
        s.into()
    }
}

/// Support python-ish "{:.2f}" or plain "%.2f"-style precision specs minimally.
fn format_float_spec(f: f64, spec: &str) -> CompactString {
    if let Some(p) = spec
        .strip_prefix("{:.")
        .and_then(|s| s.strip_suffix("f}"))
        .and_then(|s| s.parse::<usize>().ok())
    {
        return format!("{:.*}", p, f).into();
    }
    if let Some(p) = spec
        .strip_prefix("%.")
        .and_then(|s| s.strip_suffix('f'))
        .and_then(|s| s.parse::<usize>().ok())
    {
        return format!("{:.*}", p, f).into();
    }
    format_float(f)
}

const EPOCH_FORMATS: &[&str] = &[
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S%.f%z",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y",
    "%m/%d/%y",
    "%d %b %Y",
    "%d %B %Y",
    "%b %d, %Y",
    "%B %d, %Y",
    "%Y%m%d",
    "%H:%M:%S",
];

/// Parse a date string via the format ladder; returns epoch micros.
pub fn parse_date(s: &str) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // RFC3339 fast path
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.timestamp_micros());
    }
    for fmt in EPOCH_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(dt.and_utc().timestamp_micros());
        }
        if let Ok(d) = NaiveDate::parse_from_str(t, fmt) {
            return Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_micros());
        }
        if let Ok(tm) = NaiveTime::parse_from_str(t, fmt) {
            let d = NaiveDate::from_ymd_opt(1970, 1, 1)?;
            return Some(d.and_time(tm).and_utc().timestamp_micros());
        }
    }
    None
}

pub fn format_date(us: i64, fmt: &str) -> CompactString {
    use chrono::DateTime;
    match DateTime::from_timestamp_micros(us) {
        Some(dt) => {
            let naive = dt.naive_utc();
            // trim time-of-midnight for date-only values when using default fmt
            if fmt == "%Y-%m-%d %H:%M:%S" && us % 86_400_000_000 == 0 {
                return naive.format("%Y-%m-%d").to_compact_string();
            }
            naive.format(fmt).to_compact_string()
        }
        None => "<invalid date>".into(),
    }
}

/// Order two values for sorting: Null < numbers/dates < strings < errors.
pub fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    use Value::*;
    fn rank(v: &Value) -> u8 {
        match v {
            Null => 0,
            Bool(_) | Int(_) | Float(_) | Date(_) => 1,
            Str(_) => 2,
            Error(_) => 3,
        }
    }
    match (a, b) {
        (Null, Null) => Equal,
        (Str(x), Str(y)) => x.cmp(y),
        (Error(x), Error(y)) => x.cmp(y),
        (x, y) if rank(x) == 1 && rank(y) == 1 => {
            let xf = x.as_f64().unwrap_or(f64::NAN);
            let yf = y.as_f64().unwrap_or(f64::NAN);
            xf.partial_cmp(&yf).unwrap_or(Equal)
        }
        (x, y) => rank(x).cmp(&rank(y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_parsing() {
        assert_eq!(parse_int_str("42"), Some(42));
        assert_eq!(parse_int_str(" 1,234 "), Some(1234));
        assert_eq!(parse_int_str("3.0"), Some(3));
        assert_eq!(parse_int_str("abc"), None);
        assert_eq!(parse_int_str("3.5"), None);
    }

    #[test]
    fn float_parsing() {
        assert_eq!(parse_float_str("3.5"), Some(3.5));
        assert_eq!(parse_float_str("1,234.5"), Some(1234.5));
        assert_eq!(parse_float_str("50%"), Some(0.5));
        assert_eq!(parse_float_str("1e3"), Some(1000.0));
    }

    #[test]
    fn floatsi_parsing() {
        assert_eq!(parse_floatsi("1.5M"), Some(1_500_000.0));
        assert_eq!(parse_floatsi("2k"), Some(2000.0));
        assert_eq!(parse_floatsi("3"), Some(3.0));
    }

    #[test]
    fn date_ladder() {
        assert!(parse_date("2024-01-15").is_some());
        assert!(parse_date("2024-01-15 10:30:00").is_some());
        assert!(parse_date("01/15/2024").is_some());
        assert!(parse_date("Jan 15, 2024").is_some());
        assert!(parse_date("garbage").is_none());
        let us = parse_date("1970-01-02").unwrap();
        assert_eq!(us, 86_400_000_000);
    }

    #[test]
    fn date_display_roundtrip() {
        let us = parse_date("2024-01-15").unwrap();
        assert_eq!(format_date(us, "%Y-%m-%d %H:%M:%S"), "2024-01-15");
    }

    #[test]
    fn type_coercion() {
        assert_eq!(VType::Int.parse("42"), Value::Int(42));
        assert_eq!(VType::Float.parse("3.5"), Value::Float(3.5));
        assert!(VType::Int.parse("xyz").is_error());
        assert_eq!(VType::Any.parse(""), Value::Null);
        assert_eq!(VType::Len.parse("héllo"), Value::Int(5));
    }

    #[test]
    fn currency_format() {
        let v = Value::Float(1234567.891);
        assert_eq!(VType::Currency.format(&v, None), "1,234,567.89");
    }

    #[test]
    fn ordering() {
        use std::cmp::Ordering::*;
        assert_eq!(value_cmp(&Value::Null, &Value::Int(1)), Less);
        assert_eq!(value_cmp(&Value::Int(2), &Value::Float(1.5)), Greater);
        assert_eq!(value_cmp(&Value::Int(1), &Value::Str("a".into())), Less);
        assert_eq!(
            value_cmp(&Value::Str("a".into()), &Value::Str("b".into())),
            Less
        );
    }
}
