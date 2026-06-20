//! Mapping caller *intent* (locale + ISO-4217 code, decimals, negative style) to
//! the OOXML number-format code the cell carries — and the ISO-date → Excel
//! serial conversion dates need to be stored as real, sortable numbers.
//!
//! The library is country-agnostic: nothing here hardcodes a single currency or
//! locale. `locale` + `code` are inputs; an unknown pairing falls back to a
//! sensible default (symbol = the ISO code, prefix placement) rather than failing.

use crate::model::{CurrencyFormat, DateFormat, DateKind, Negative, NumberFormat};

/// A resolved number format: either a built-in OOXML format id (no `numFmt`
/// entry needed) or a custom format code that the style table registers under a
/// fresh id (164+).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedFmt {
    /// A built-in OOXML `numFmtId` (e.g. 14 = locale short date).
    Builtin(u32),
    /// A custom format code, e.g. `"\"$\"#,##0.00;[Red](\"$\"#,##0.00)"`.
    Custom(String),
}

/// Resolve a currency cell's format. `fallback_locale` is the sheet/workbook
/// locale used when the `CurrencyFormat` omits its own.
pub fn currency(fmt: &CurrencyFormat, fallback_locale: &str) -> ResolvedFmt {
    let decimals = fmt.decimals.unwrap_or(2);
    let locale = fmt.locale.as_deref().unwrap_or(fallback_locale);
    let token = if fmt.symbol == Some(false) {
        format!("\"{} \"", fmt.code)
    } else {
        format!("\"{}\"", symbol_for(&fmt.code))
    };
    let body = grouped_body(decimals, true);
    let positive = place_symbol(&token, &body, is_suffix(locale));
    ResolvedFmt::Custom(with_negative(&positive, fmt.negative))
}

/// Resolve a plain-number cell's format. A `raw` code short-circuits everything.
pub fn number(fmt: &NumberFormat) -> ResolvedFmt {
    if let Some(raw) = &fmt.raw {
        return ResolvedFmt::Custom(raw.clone());
    }
    let decimals = fmt.decimals.unwrap_or(0);
    let grouped = fmt.grouped.unwrap_or(true);
    let body = grouped_body(decimals, grouped);
    ResolvedFmt::Custom(with_negative(&body, fmt.negative))
}

/// Resolve a percent cell's format from its decimal count (default 2).
pub fn percent(decimals: u32) -> ResolvedFmt {
    match decimals {
        0 => ResolvedFmt::Builtin(9),
        2 => ResolvedFmt::Builtin(10),
        n => ResolvedFmt::Custom(format!("0.{}%", "0".repeat(n as usize))),
    }
}

/// Resolve a date cell's format. A `raw` code wins; otherwise the semantic kind
/// maps to a built-in id (14 short date, 22 datetime, 17 `mmm-yy`).
pub fn date(fmt: Option<&DateFormat>) -> ResolvedFmt {
    let Some(fmt) = fmt else {
        return ResolvedFmt::Builtin(14);
    };
    if let Some(raw) = &fmt.raw {
        return ResolvedFmt::Custom(raw.clone());
    }
    match fmt.kind.unwrap_or(DateKind::Date) {
        DateKind::Date => ResolvedFmt::Builtin(14),
        DateKind::Datetime => ResolvedFmt::Builtin(22),
        DateKind::MonthYear => ResolvedFmt::Builtin(17),
    }
}

/// The integer + optional fractional placeholder body, e.g. `#,##0.00` / `0`.
fn grouped_body(decimals: u32, grouped: bool) -> String {
    let integer = if grouped { "#,##0" } else { "0" };
    if decimals == 0 {
        integer.to_string()
    } else {
        format!("{integer}.{}", "0".repeat(decimals as usize))
    }
}

/// Place the currency token before or after the number body.
fn place_symbol(token: &str, body: &str, suffix: bool) -> String {
    if suffix {
        format!("{body}{token}")
    } else {
        format!("{token}{body}")
    }
}

/// Wrap a positive-section pattern with the negative section per the convention.
fn with_negative(positive: &str, negative: Option<Negative>) -> String {
    match negative.unwrap_or(Negative::Minus) {
        Negative::Minus => positive.to_string(),
        Negative::Red => format!("{positive};[Red]-{positive}"),
        Negative::Parens => format!("{positive};({positive})"),
        Negative::RedParens => format!("{positive};[Red]({positive})"),
    }
}

/// The currency symbol for an ISO-4217 code; the code itself when unknown.
fn symbol_for(code: &str) -> &str {
    match code {
        "USD" | "MXN" | "CAD" | "AUD" | "ARS" | "CLP" | "COP" => "$",
        "EUR" => "\u{20ac}",
        "GBP" => "\u{a3}",
        "JPY" | "CNY" => "\u{a5}",
        "BRL" => "R$",
        other => other,
    }
}

/// Whether `locale`'s region places the currency symbol after the amount. Most
/// European regions suffix (`1 234,56 \u{20ac}`); the Americas prefix (`$1,234.56`).
fn is_suffix(locale: &str) -> bool {
    const SUFFIX_REGIONS: &[&str] = &[
        "ES", "PT", "FR", "DE", "IT", "NL", "BE", "AT", "IE", "FI", "GR", "PL", "SE", "DK", "CZ",
        "HU", "RO", "NO", "CH",
    ];
    match locale.split('-').nth(1) {
        Some(region) => SUFFIX_REGIONS.contains(&region),
        None => false,
    }
}

/// Convert a date cell value to an Excel serial number. An ISO string is parsed;
/// a string that does not parse yields `None` (the caller renders it as text).
pub fn iso_to_serial(s: &str) -> Option<f64> {
    let (date_part, time_part) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let days = parse_date(date_part)?;
    let frac = match time_part {
        Some(t) => parse_time(t)?,
        None => 0.0,
    };
    Some((days + 25_569) as f64 + frac)
}

/// Parse `YYYY-MM-DD` into days since the Unix epoch (1970-01-01).
fn parse_date(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s
        .split('-')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [y, m, d] = parts[..] else {
        return None;
    };
    valid_ymd(m, d).then(|| days_from_civil(y, m, d))
}

/// Whether a month/day pair falls in the (loose) valid calendar range.
fn valid_ymd(m: i64, d: i64) -> bool {
    (1..=12).contains(&m) && (1..=31).contains(&d)
}

/// Parse `HH:MM` or `HH:MM:SS` into a day fraction in `[0, 1)`.
fn parse_time(s: &str) -> Option<f64> {
    let parts: Vec<f64> = s
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let (h, m, sec) = match parts[..] {
        [h, m] => (h, m, 0.0),
        [h, m, sec] => (h, m, sec),
        _ => return None,
    };
    Some((h * 3600.0 + m * 60.0 + sec) / 86_400.0)
}

/// Howard Hinnant's `days_from_civil`: days from 1970-01-01 to `y-m-d`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CurrencyFormat, DateFormat, DateKind, Negative, NumberFormat};

    fn cf(
        code: &str,
        locale: Option<&str>,
        neg: Option<Negative>,
        symbol: Option<bool>,
        dec: Option<u32>,
    ) -> CurrencyFormat {
        CurrencyFormat {
            code: code.to_string(),
            locale: locale.map(str::to_string),
            decimals: dec,
            negative: neg,
            symbol,
        }
    }

    fn custom(f: ResolvedFmt) -> String {
        match f {
            ResolvedFmt::Custom(c) => c,
            ResolvedFmt::Builtin(id) => panic!("expected custom, got builtin {id}"),
        }
    }

    #[test]
    fn currency_prefix_and_negatives() {
        assert_eq!(
            custom(currency(
                &cf("USD", Some("en-US"), None, None, None),
                "en-US"
            )),
            "\"$\"#,##0.00"
        );
        assert_eq!(
            custom(currency(
                &cf("USD", None, Some(Negative::Red), None, None),
                "en-US"
            )),
            "\"$\"#,##0.00;[Red]-\"$\"#,##0.00"
        );
        assert_eq!(
            custom(currency(
                &cf("USD", None, Some(Negative::Parens), None, None),
                "en-US"
            )),
            "\"$\"#,##0.00;(\"$\"#,##0.00)"
        );
        assert_eq!(
            custom(currency(
                &cf("USD", None, Some(Negative::RedParens), None, None),
                "en-US"
            )),
            "\"$\"#,##0.00;[Red](\"$\"#,##0.00)"
        );
        assert_eq!(
            custom(currency(
                &cf("USD", None, Some(Negative::Minus), None, None),
                "en-US"
            )),
            "\"$\"#,##0.00"
        );
    }

    #[test]
    fn currency_suffix_locale_and_symbol_off() {
        assert_eq!(
            custom(currency(
                &cf("EUR", Some("pt-PT"), None, None, None),
                "en-US"
            )),
            "#,##0.00\"\u{20ac}\""
        );
        assert_eq!(
            custom(currency(
                &cf("MXN", None, None, Some(false), Some(0)),
                "es-MX"
            )),
            "\"MXN \"#,##0"
        );
    }

    #[test]
    fn currency_symbols_cover_table() {
        assert_eq!(symbol_for("GBP"), "\u{a3}");
        assert_eq!(symbol_for("JPY"), "\u{a5}");
        assert_eq!(symbol_for("CNY"), "\u{a5}");
        assert_eq!(symbol_for("BRL"), "R$");
        assert_eq!(symbol_for("XYZ"), "XYZ");
    }

    #[test]
    fn suffix_detection() {
        assert!(is_suffix("pt-PT"));
        assert!(!is_suffix("es-MX"));
        assert!(!is_suffix("en"));
    }

    #[test]
    fn number_formats() {
        assert_eq!(
            custom(number(&NumberFormat {
                raw: Some("0.0".into()),
                ..Default::default()
            })),
            "0.0"
        );
        assert_eq!(custom(number(&NumberFormat::default())), "#,##0");
        assert_eq!(
            custom(number(&NumberFormat {
                decimals: Some(2),
                grouped: Some(false),
                ..Default::default()
            })),
            "0.00"
        );
        assert_eq!(
            custom(number(&NumberFormat {
                decimals: Some(2),
                negative: Some(Negative::Red),
                ..Default::default()
            })),
            "#,##0.00;[Red]-#,##0.00"
        );
    }

    #[test]
    fn percent_formats() {
        assert_eq!(percent(0), ResolvedFmt::Builtin(9));
        assert_eq!(percent(2), ResolvedFmt::Builtin(10));
        assert_eq!(custom(percent(3)), "0.000%");
    }

    #[test]
    fn date_formats() {
        assert_eq!(date(None), ResolvedFmt::Builtin(14));
        assert_eq!(
            date(Some(&DateFormat {
                kind: None,
                raw: Some("dd/mm/yyyy".into())
            })),
            ResolvedFmt::Custom("dd/mm/yyyy".into())
        );
        assert_eq!(
            date(Some(&DateFormat {
                kind: Some(DateKind::Date),
                raw: None
            })),
            ResolvedFmt::Builtin(14)
        );
        assert_eq!(
            date(Some(&DateFormat {
                kind: Some(DateKind::Datetime),
                raw: None
            })),
            ResolvedFmt::Builtin(22)
        );
        assert_eq!(
            date(Some(&DateFormat {
                kind: Some(DateKind::MonthYear),
                raw: None
            })),
            ResolvedFmt::Builtin(17)
        );
    }

    #[test]
    fn iso_serial_conversions() {
        assert_eq!(iso_to_serial("2024-01-01"), Some(45292.0));
        assert_eq!(iso_to_serial("1900-03-01"), Some(61.0));
        let dt = iso_to_serial("2024-01-01T12:00:00").unwrap();
        assert!((dt - 45292.5).abs() < 1e-9);
        let hm = iso_to_serial("2024-01-01T06:00").unwrap();
        assert!((hm - 45292.25).abs() < 1e-9);
    }

    #[test]
    fn iso_serial_rejects_bad_input() {
        assert_eq!(iso_to_serial("not-a-date"), None);
        assert_eq!(iso_to_serial("2024-13-01"), None);
        assert_eq!(iso_to_serial("2024-01-40"), None);
        assert_eq!(iso_to_serial("2024-01-01-01"), None);
        assert_eq!(iso_to_serial("2024-01-01T06:00:00:00"), None);
        assert_eq!(iso_to_serial("2024-01"), None);
        assert_eq!(iso_to_serial("2024-01-01Txx:00"), None);
    }
}
