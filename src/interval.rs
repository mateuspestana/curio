//! Parse human interval strings like `10s`, `50m`, `2d`, `1w`.
//! Author: Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

use crate::model::MIN_INTERVAL_SECS;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntervalError {
    #[error("intervalo vazio")]
    Empty,
    #[error("formato inválido: use N + unidade (s, m, h, d, w), ex.: 10s, 50m, 2d")]
    InvalidFormat,
    #[error("quantidade inválida")]
    InvalidAmount,
    #[error("intervalo mínimo é {MIN_INTERVAL_SECS} segundos")]
    TooSmall,
}

/// Parse `10s`, `50m`, `2h`, `2d`, `1w` into seconds.
pub fn parse_interval(input: &str) -> Result<u64, IntervalError> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err(IntervalError::Empty);
    }

    let split = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(i, _)| i);

    let Some(idx) = split else {
        return Err(IntervalError::InvalidFormat);
    };

    if idx == 0 {
        return Err(IntervalError::InvalidFormat);
    }

    let (num_part, unit_part) = s.split_at(idx);
    if unit_part.chars().count() != 1 {
        return Err(IntervalError::InvalidFormat);
    }

    let amount: u64 = num_part.parse().map_err(|_| IntervalError::InvalidAmount)?;
    if amount == 0 {
        return Err(IntervalError::InvalidAmount);
    }

    let unit = unit_part.chars().next().unwrap();
    let secs = match unit {
        's' => amount,
        'm' => amount.checked_mul(60).ok_or(IntervalError::InvalidAmount)?,
        'h' => amount
            .checked_mul(3600)
            .ok_or(IntervalError::InvalidAmount)?,
        'd' => amount
            .checked_mul(86400)
            .ok_or(IntervalError::InvalidAmount)?,
        'w' => amount
            .checked_mul(604800)
            .ok_or(IntervalError::InvalidAmount)?,
        _ => return Err(IntervalError::InvalidFormat),
    };

    if secs < MIN_INTERVAL_SECS {
        return Err(IntervalError::TooSmall);
    }

    Ok(secs)
}

/// Format seconds as a compact human string (largest fitting unit when exact).
pub fn format_interval(secs: u64) -> String {
    if secs.is_multiple_of(604800) {
        format!("{}w", secs / 604800)
    } else if secs.is_multiple_of(86400) {
        format!("{}d", secs / 86400)
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_units() {
        assert_eq!(parse_interval("10s").unwrap(), 10);
        assert_eq!(parse_interval("50m").unwrap(), 3000);
        assert_eq!(parse_interval("2h").unwrap(), 7200);
        assert_eq!(parse_interval("2d").unwrap(), 172800);
        assert_eq!(parse_interval("1w").unwrap(), 604800);
        assert_eq!(parse_interval(" 5S ").unwrap(), 5);
    }

    #[test]
    fn rejects_too_small() {
        assert_eq!(parse_interval("4s").unwrap_err(), IntervalError::TooSmall);
    }

    #[test]
    fn rejects_invalid() {
        assert!(parse_interval("").is_err());
        assert!(parse_interval("abc").is_err());
        assert!(parse_interval("10").is_err());
        assert!(parse_interval("10x").is_err());
        assert!(parse_interval("0s").is_err());
    }

    #[test]
    fn formats_roundtrip_friendly() {
        assert_eq!(format_interval(10), "10s");
        assert_eq!(format_interval(3000), "50m");
        assert_eq!(format_interval(172800), "2d");
    }
}
