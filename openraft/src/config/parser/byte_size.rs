//! Byte-size parsing for the CLI, such as `204`, `3MiB` or `5.3 KB`.
//!
//! It accepts the grammar of the default configuration of `parse-size` 1.1.0, which it replaces:
//! that crate rounds a mantissa that does not fit `u64` by adding 1 to it, which overflows when
//! the mantissa is already `u64::MAX`. The overflow panicked in a debug build, and parsed as 0 in a
//! release build. Here the mantissa is a `u128`, so that rounding gives 2^64, which the final
//! scaling either brings back into range or reports as too large.

use std::fmt;

/// Why a byte size failed to parse. The messages are those of `str::parse::<u64>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseSizeError {
    /// The input holds no digits.
    Empty,
    /// The input holds a character that the grammar does not accept there.
    InvalidDigit,
    /// The size does not fit `u64`.
    PosOverflow,
}

impl fmt::Display for ParseSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "cannot parse integer from empty string",
            Self::InvalidDigit => "invalid digit found in string",
            Self::PosOverflow => "number too large to fit in target type",
        })
    }
}

/// Parses `src` into a number of bytes.
///
/// A number, with `_` or space separators, a fraction and an exponent such as `1.5e3`, is
/// followed by an optional unit: a prefix `k`, `m`, `g`, `t`, `p` or `e`, in powers of 1000, or
/// with `i` in powers of 1024, then an optional `B`, in any letter case. The result is rounded to
/// the nearest byte.
pub(crate) fn parse_size(src: &str) -> Result<u64, ParseSizeError> {
    let mut src = src.as_bytes();

    if let [init @ .., b'b' | b'B'] = src {
        src = init;
    }

    let binary = if let [init @ .., b'i' | b'I'] = src {
        src = init;
        true
    } else {
        false
    };

    let mut multiply = 1;
    if let [init @ .., prefix] = src
        && let Some(factor) = unit_factor(*prefix, binary)
    {
        multiply = factor;
        src = init;
    }

    parse_with_multiple(src, multiply)
}

/// The factor of a unit prefix, in powers of 1000, or of 1024 when `binary`.
fn unit_factor(prefix: u8, binary: bool) -> Option<u64> {
    let power = match prefix.to_ascii_lowercase() {
        b'k' => 1,
        b'm' => 2,
        b'g' => 3,
        b't' => 4,
        b'p' => 5,
        b'e' => 6,
        _ => return None,
    };
    Some(if binary { 1 << (10 * power) } else { 1000_u64.pow(power) })
}

/// Parses the number in `src` and multiplies it by `multiply`, rounding to the nearest integer.
fn parse_with_multiple(src: &[u8], multiply: u64) -> Result<u64, ParseSizeError> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Empty,
        Integer,
        IntegerOverflow,
        Fraction,
        FractionOverflow,
        PosExponent,
        NegExponent,
    }

    // Digits accumulate while the mantissa fits `u64`. The first digit that does not fit rounds
    // the mantissa, which can make it 2^64, and every later one only moves the decimal point.
    let mut mantissa = 0_u128;
    let mut fractional_exponent = 0_i32;
    let mut exponent = 0_i32;
    let mut state = State::Empty;

    let append = |mantissa: u128, digit: u8| -> Option<u128> {
        let appended = mantissa * 10 + u128::from(digit - b'0');
        (appended <= u128::from(u64::MAX)).then_some(appended)
    };
    let round_up = |digit: u8| u128::from(digit >= b'5');

    for &b in src {
        match (state, b) {
            (State::Empty | State::Integer, b'0'..=b'9') => {
                if let Some(appended) = append(mantissa, b) {
                    mantissa = appended;
                    state = State::Integer;
                } else {
                    mantissa += round_up(b);
                    fractional_exponent += 1;
                    state = State::IntegerOverflow;
                }
            }
            (State::IntegerOverflow, b'0'..=b'9') => fractional_exponent += 1,
            (State::Fraction, b'0'..=b'9') => {
                if let Some(appended) = append(mantissa, b) {
                    mantissa = appended;
                    fractional_exponent -= 1;
                } else {
                    mantissa += round_up(b);
                    state = State::FractionOverflow;
                }
            }
            (State::PosExponent, b'0'..=b'9') => {
                let digit = i32::from(b - b'0');
                let appended = exponent.checked_mul(10).and_then(|e| e.checked_add(digit));
                exponent = appended.ok_or(ParseSizeError::PosOverflow)?;
            }
            (State::NegExponent, b'0'..=b'9') => {
                let digit = i32::from(b - b'0');
                // A negative exponent that no longer fits `i32` stays where it was: the size
                // rounds to 0 long before that.
                if let Some(appended) = exponent.checked_mul(10).and_then(|e| e.checked_sub(digit)) {
                    exponent = appended;
                }
            }
            (_, b'_' | b' ') | (State::PosExponent, b'+') | (State::FractionOverflow, b'0'..=b'9') => {}
            (State::Integer | State::Fraction | State::IntegerOverflow | State::FractionOverflow, b'e' | b'E') => {
                state = State::PosExponent;
            }
            (State::PosExponent, b'-') => state = State::NegExponent,
            (State::Integer, b'.') => state = State::Fraction,
            (State::IntegerOverflow, b'.') => state = State::FractionOverflow,
            _ => return Err(ParseSizeError::InvalidDigit),
        }
    }

    if state == State::Empty {
        return Err(ParseSizeError::Empty);
    }

    let exponent = exponent.saturating_add(fractional_exponent);
    let abs_exponent = exponent.unsigned_abs();
    let size = if exponent >= 0 {
        let power = 10_u64.checked_pow(abs_exponent).ok_or(ParseSizeError::PosOverflow)?;
        let multiply = multiply.checked_mul(power).ok_or(ParseSizeError::PosOverflow)?;
        mantissa.checked_mul(u128::from(multiply)).ok_or(ParseSizeError::PosOverflow)?
    } else if exponent >= -38 {
        // At most 2^64 * 2^60 + 10^38 / 2, which fits `u128`.
        let power = 10_u128.pow(abs_exponent);
        (mantissa * u128::from(multiply) + power / 2) / power
    } else {
        // 2^128 * 10^-39 < 1, so the size rounds to 0.
        0
    };
    u64::try_from(size).map_err(|_| ParseSizeError::PosOverflow)
}

#[cfg(test)]
mod byte_size_test;
