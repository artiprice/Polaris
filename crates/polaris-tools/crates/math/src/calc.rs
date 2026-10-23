//! Bounded decimal and floating-point expression evaluators.

use core::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::MathError;

/// Bounds for expression parsing and exponentiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcLimits {
    /// Maximum expression byte length.
    pub max_len: usize,
    /// Maximum nested parse depth.
    pub max_depth: u8,
    /// Maximum absolute integer exponent.
    pub max_pow_exponent: u32,
}

impl Default for CalcLimits {
    fn default() -> Self {
        Self {
            max_len: 1024,
            max_depth: 64,
            max_pow_exponent: 1000,
        }
    }
}

/// Evaluates a restricted decimal expression using checked arithmetic only.
///
/// Supported operators are `+`, `-`, `*`, `/`, `%`, and integer `^`; unary signs,
/// parentheses, and `abs`, `min`, `max`, `round`, `floor`, and `ceil` are supported.
/// Decimal division uses the fixed precision and range of `rust_decimal`.
pub fn calc(expr: &str, limits: &CalcLimits) -> Result<Decimal, MathError> {
    if expr.len() > limits.max_len || limits.max_len == 0 {
        return Err(MathError::TooLong);
    }
    if limits.max_depth == 0 {
        return Err(MathError::TooDeep);
    }
    let mut parser = Parser {
        bytes: expr.as_bytes(),
        position: 0,
        limits: *limits,
    };
    let value = parser.expression(0, 0)?;
    parser.skip_space();
    if parser.position != parser.bytes.len() {
        return Err(MathError::Parse {
            at: parser.position,
        });
    }
    Ok(value)
}

/// Evaluates a bounded float expression using exmex's built-in transcendental operators.
///
/// Variables are rejected. Allowed names are `sin`, `cos`, `tan`, `exp`, `ln`, `log10`,
/// `log2`, `sqrt`, `abs`, `PI`, `TAU`, and `E`.
pub fn calc_f64(expr: &str, limits: &CalcLimits) -> Result<f64, MathError> {
    validate_expression_shape(expr, limits)?;
    validate_float_exponents(expr, limits)?;
    validate_float_names(expr)?;
    let value = exmex::eval_str::<f64>(expr).map_err(|_| MathError::Parse { at: 0 })?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(MathError::NonFinite)
    }
}

fn validate_float_exponents(expr: &str, limits: &CalcLimits) -> Result<(), MathError> {
    let bytes = expr.as_bytes();
    let mut cursor = 0_usize;
    while let Some(byte) = bytes.get(cursor).copied() {
        if byte == b'^' {
            let mut start = cursor.checked_add(1).ok_or(MathError::TooLong)?;
            while bytes
                .get(start)
                .is_some_and(|value| value.is_ascii_whitespace())
            {
                start = start.checked_add(1).ok_or(MathError::TooLong)?;
            }
            let signed_start = start;
            if bytes
                .get(start)
                .is_some_and(|value| *value == b'+' || *value == b'-')
            {
                start = start.checked_add(1).ok_or(MathError::TooLong)?;
            }
            let digits_start = start;
            while bytes.get(start).is_some_and(u8::is_ascii_digit) {
                start = start.checked_add(1).ok_or(MathError::TooLong)?;
            }
            if digits_start < start && bytes.get(start) != Some(&b'.') {
                let token = core::str::from_utf8(
                    bytes
                        .get(signed_start..start)
                        .ok_or(MathError::Parse { at: signed_start })?,
                )
                .map_err(|_| MathError::Parse { at: signed_start })?;
                let exponent = token.parse::<i64>().map_err(|_| MathError::ExponentLimit)?;
                if exponent.unsigned_abs() > u64::from(limits.max_pow_exponent) {
                    return Err(MathError::ExponentLimit);
                }
            }
        }
        cursor = cursor.checked_add(1).ok_or(MathError::TooLong)?;
    }
    Ok(())
}

fn validate_expression_shape(expr: &str, limits: &CalcLimits) -> Result<(), MathError> {
    if expr.len() > limits.max_len || limits.max_len == 0 {
        return Err(MathError::TooLong);
    }
    let mut depth = 0_u8;
    for byte in expr.bytes() {
        match byte {
            b'(' => {
                depth = depth.checked_add(1).ok_or(MathError::TooDeep)?;
                if depth > limits.max_depth {
                    return Err(MathError::TooDeep);
                }
            }
            b')' => {
                if depth == 0 {
                    return Err(MathError::Parse { at: expr.len() });
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    if depth > 0 {
        return Err(MathError::Parse { at: expr.len() });
    }
    Ok(())
}

fn validate_float_names(expr: &str) -> Result<(), MathError> {
    let bytes = expr.as_bytes();
    let mut cursor = 0_usize;
    while let Some(byte) = bytes.get(cursor).copied() {
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = cursor;
            cursor = cursor.checked_add(1).ok_or(MathError::TooLong)?;
            while bytes
                .get(cursor)
                .is_some_and(|value| value.is_ascii_alphanumeric() || *value == b'_')
            {
                cursor = cursor.checked_add(1).ok_or(MathError::TooLong)?;
            }
            let name = core::str::from_utf8(
                bytes
                    .get(start..cursor)
                    .ok_or(MathError::Parse { at: start })?,
            )
            .map_err(|_| MathError::Parse { at: start })?;
            let lower = matches!(
                name,
                "sin" | "cos" | "tan" | "exp" | "ln" | "log10" | "log2" | "sqrt" | "abs"
            );
            let constant = matches!(name, "PI" | "TAU" | "E");
            if !lower && !constant {
                return Err(MathError::Parse { at: start });
            }
        } else {
            cursor = cursor.checked_add(1).ok_or(MathError::TooLong)?;
        }
    }
    Ok(())
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    limits: CalcLimits,
}

impl Parser<'_> {
    fn expression(&mut self, min_binding: u8, depth: u8) -> Result<Decimal, MathError> {
        if depth > self.limits.max_depth {
            return Err(MathError::TooDeep);
        }
        self.skip_space();
        let nested = depth.checked_add(1).ok_or(MathError::TooDeep)?;
        let mut left = match self.peek() {
            Some(b'+') => {
                self.advance()?;
                self.expression(5, nested)?
            }
            Some(b'-') => {
                self.advance()?;
                Decimal::ZERO
                    .checked_sub(self.expression(5, nested)?)
                    .ok_or(MathError::Overflow)?
            }
            Some(b'(') => {
                self.advance()?;
                let value = self.expression(0, nested)?;
                self.skip_space();
                if self.peek() != Some(b')') {
                    return Err(MathError::Parse { at: self.position });
                }
                self.advance()?;
                value
            }
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => self.number()?,
            Some(byte) if byte.is_ascii_alphabetic() => self.function(nested)?,
            _ => {
                return Err(MathError::Parse { at: self.position });
            }
        };

        loop {
            self.skip_space();
            let (left_binding, right_binding) = match self.peek() {
                Some(b'+') | Some(b'-') => (1, 2),
                Some(b'*') | Some(b'/') | Some(b'%') => (3, 4),
                Some(b'^') => (6, 6),
                _ => break,
            };
            if left_binding < min_binding {
                break;
            }
            let operator = self
                .advance()?
                .ok_or(MathError::Parse { at: self.position })?;
            let right = self.expression(right_binding, nested)?;
            left = match operator {
                b'+' => left.checked_add(right).ok_or(MathError::Overflow)?,
                b'-' => left.checked_sub(right).ok_or(MathError::Overflow)?,
                b'*' => left.checked_mul(right).ok_or(MathError::Overflow)?,
                b'/' => {
                    if right.is_zero() {
                        return Err(MathError::DivideByZero);
                    }
                    left.checked_div(right).ok_or(MathError::Overflow)?
                }
                b'%' => {
                    if right.is_zero() {
                        return Err(MathError::DivideByZero);
                    }
                    left.checked_rem(right).ok_or(MathError::Overflow)?
                }
                b'^' => {
                    if !right.fract().is_zero() {
                        return Err(MathError::Parse { at: self.position });
                    }
                    let exponent = right.to_i64().ok_or(MathError::ExponentLimit)?;
                    if exponent.unsigned_abs() > u64::from(self.limits.max_pow_exponent) {
                        return Err(MathError::ExponentLimit);
                    }
                    checked_integer_power(left, exponent)?
                }
                _ => return Err(MathError::Parse { at: self.position }),
            };
        }
        Ok(left)
    }

    fn function(&mut self, depth: u8) -> Result<Decimal, MathError> {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        {
            self.advance()?;
        }
        let name = core::str::from_utf8(
            self.bytes
                .get(start..self.position)
                .ok_or(MathError::Parse { at: start })?,
        )
        .map_err(|_| MathError::Parse { at: start })?;
        self.skip_space();
        if self.peek() != Some(b'(') {
            return Err(MathError::Parse { at: start });
        }
        self.advance()?;
        let first = self.expression(0, depth)?;
        self.skip_space();
        let second = if self.peek() == Some(b',') {
            self.advance()?;
            Some(self.expression(0, depth)?)
        } else {
            None
        };
        self.skip_space();
        if self.peek() != Some(b')') {
            return Err(MathError::Parse { at: self.position });
        }
        self.advance()?;
        match (name, second) {
            ("abs", None) => {
                if first.is_sign_negative() {
                    Decimal::ZERO.checked_sub(first).ok_or(MathError::Overflow)
                } else {
                    Ok(first)
                }
            }
            ("round", None) => Ok(first.round_dp(0)),
            ("floor", None) => Ok(first.floor()),
            ("ceil", None) => Ok(first.ceil()),
            ("min", Some(second)) => Ok(first.min(second)),
            ("max", Some(second)) => Ok(first.max(second)),
            _ => Err(MathError::Parse { at: start }),
        }
    }

    fn number(&mut self) -> Result<Decimal, MathError> {
        let start = self.position;
        let mut saw_digit = false;
        let mut saw_point = false;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() {
                saw_digit = true;
                self.advance()?;
            } else if byte == b'.' && !saw_point {
                saw_point = true;
                self.advance()?;
            } else {
                break;
            }
        }
        if !saw_digit {
            return Err(MathError::Parse { at: start });
        }
        let text = core::str::from_utf8(
            self.bytes
                .get(start..self.position)
                .ok_or(MathError::Parse { at: start })?,
        )
        .map_err(|_| MathError::Parse { at: start })?;
        Decimal::from_str(text).map_err(|_| MathError::Parse { at: start })
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position = self.position.saturating_add(1);
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn advance(&mut self) -> Result<Option<u8>, MathError> {
        let value = self.peek();
        if value.is_some() {
            self.position = self.position.checked_add(1).ok_or(MathError::TooLong)?;
        }
        Ok(value)
    }
}

fn checked_integer_power(base: Decimal, exponent: i64) -> Result<Decimal, MathError> {
    if exponent < 0 && base.is_zero() {
        return Err(MathError::DivideByZero);
    }
    let mut power = exponent.unsigned_abs();
    let mut factor = base;
    let mut product = Decimal::ONE;
    while power > 0 {
        if power & 1 == 1 {
            product = product.checked_mul(factor).ok_or(MathError::Overflow)?;
        }
        power >>= 1;
        if power > 0 {
            factor = factor.checked_mul(factor).ok_or(MathError::Overflow)?;
        }
    }
    if exponent < 0 {
        Decimal::ONE.checked_div(product).ok_or(MathError::Overflow)
    } else {
        Ok(product)
    }
}
