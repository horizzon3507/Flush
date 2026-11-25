//! Exact decimal arithmetic in text form (yuzuha `domain/decimal.ts`).

use num_bigint::BigInt;
use num_traits::{Signed, Zero};

use crate::errors::FnError;

const MAX_DIGITS: usize = 1000;

#[derive(Clone)]
struct Decimal {
    coefficient: BigInt,
    scale: i64,
}

fn decimal_error(message: &str) -> FnError {
    FnError::new("DECIMAL", message.to_string())
}

fn parse(value: &str) -> Result<Decimal, FnError> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    let (sign, rest) = match bytes.first() {
        Some(b'+') | Some(b'-') if bytes.len() > 1 => (bytes[0] == b'-', &trimmed[1..]),
        Some(b'+') | Some(b'-') => return Err(invalid()),
        _ => (false, trimmed),
    };
    let split: Vec<&str> = rest.splitn(2, '.').collect();
    let (int_part, frac_part) = match split.as_slice() {
        [int_part] => (*int_part, ""),
        [int_part, frac_part] => (*int_part, *frac_part),
        _ => return Err(invalid()),
    };
    let digits = format!("{int_part}{frac_part}");
    if int_part.is_empty()
        || !int_part.chars().all(|ch| ch.is_ascii_digit())
        || !frac_part.chars().all(|ch| ch.is_ascii_digit())
        || digits.is_empty()
    {
        return Err(invalid());
    }
    if digits.len() > MAX_DIGITS {
        return Err(FnError::new(
            "EXECUTION_LIMIT",
            format!("Decimal exceeds {MAX_DIGITS} digits."),
        ));
    }
    let coefficient: BigInt = digits.parse().map_err(|_| invalid())?;
    Ok(Decimal {
        coefficient: if sign { -coefficient } else { coefficient },
        scale: frac_part.len() as i64,
    })
}

fn invalid() -> FnError {
    decimal_error("Provide a decimal without exponent, using a dot.")
}

fn places(value: i64) -> Result<i64, FnError> {
    if value < 0 || value > MAX_DIGITS as i64 {
        return Err(decimal_error(&format!(
            "Places must be an integer from 0 to {MAX_DIGITS}."
        )));
    }
    Ok(value)
}

fn pow(exponent: i64) -> BigInt {
    BigInt::from(10u32).pow(exponent.max(0) as u32)
}

fn quotient(numerator: &BigInt, denominator: &BigInt) -> Result<BigInt, FnError> {
    if denominator.is_zero() {
        return Err(decimal_error("Division by zero."));
    }
    let result = numerator / denominator;
    if (numerator % denominator).abs() * 2u32 < denominator.abs() {
        return Ok(result);
    }
    Ok(result
        + if numerator.is_negative() != denominator.is_negative() {
            BigInt::from(-1)
        } else {
            BigInt::from(1)
        })
}

fn rounded(value: Decimal, scale: i64) -> Result<Decimal, FnError> {
    places(scale)?;
    if value.scale <= scale {
        return Ok(value);
    }
    Ok(Decimal {
        coefficient: quotient(&value.coefficient, &pow(value.scale - scale))?,
        scale,
    })
}

fn format_decimal(mut value: Decimal) -> Result<String, FnError> {
    if value.coefficient.is_zero() {
        return Ok("0".to_string());
    }
    while value.scale > 0 && &value.coefficient % 10u32 == BigInt::zero() {
        value.coefficient /= 10u32;
        value.scale -= 1;
    }
    let negative = value.coefficient.is_negative();
    let digits = value.coefficient.abs().to_string();
    let scale = value.scale as usize;
    let digits = format!("{:0>width$}", digits, width = scale + 1);
    if digits.len() > MAX_DIGITS {
        return Err(FnError::new(
            "EXECUTION_LIMIT",
            format!("Decimal result exceeds {MAX_DIGITS} digits."),
        ));
    }
    let result = if scale > 0 {
        format!(
            "{}.{}",
            &digits[..digits.len() - scale],
            &digits[digits.len() - scale..]
        )
    } else {
        digits
    };
    Ok(if negative {
        format!("-{result}")
    } else {
        result
    })
}

pub fn decimal_compare(left: &str, right: &str) -> Result<i64, FnError> {
    let a = parse(left)?;
    let b = parse(right)?;
    let scale = a.scale.max(b.scale);
    let x = &a.coefficient * pow(scale - a.scale);
    let y = &b.coefficient * pow(scale - b.scale);
    Ok(match x.cmp(&y) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => 0,
    })
}

pub fn decimal_round(input: &str, scale: i64) -> Result<String, FnError> {
    format_decimal(rounded(parse(input)?, scale)?)
}

pub fn decimal(
    left: &str,
    operator: &str,
    right: &str,
    scale: Option<i64>,
) -> Result<String, FnError> {
    let a = parse(left)?;
    let b = parse(right)?;
    if let Some(scale) = scale {
        places(scale)?;
    }
    let common = a.scale.max(b.scale);
    let x = &a.coefficient * pow(common - a.scale);
    let y = &b.coefficient * pow(common - b.scale);
    let result = match operator {
        "+" => Decimal {
            coefficient: x + y,
            scale: common,
        },
        "-" => Decimal {
            coefficient: x - y,
            scale: common,
        },
        "*" => Decimal {
            coefficient: &a.coefficient * &b.coefficient,
            scale: a.scale + b.scale,
        },
        "%" => {
            if y.is_zero() {
                return Err(decimal_error("Division by zero."));
            }
            Decimal {
                coefficient: x % y,
                scale: common,
            }
        }
        "/" => {
            let output_scale = scale.unwrap_or(18);
            let exponent = b.scale + output_scale - a.scale;
            Decimal {
                coefficient: quotient(
                    &(&a.coefficient * pow(exponent.max(0))),
                    &(&b.coefficient * pow((-exponent).max(0))),
                )?,
                scale: output_scale,
            }
        }
        _ => {
            return Err(decimal_error("Operator must be +, -, *, / or %."));
        }
    };
    match scale {
        None => format_decimal(result),
        Some(scale) => format_decimal(rounded(result, scale)?),
    }
}
