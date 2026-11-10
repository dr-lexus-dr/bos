//! Checked integer arithmetic for the built-in calculator.

pub(super) fn evaluate(input: &str) -> Result<i64, &'static str> {
    let mut parser = Parser { rest: input };
    let value = parser.expression(0, 0)?;
    if parser.peek().is_some() {
        return Err("unexpected input");
    }
    Ok(value)
}

struct Parser<'a> {
    rest: &'a str,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.rest = self.rest.trim_start();
        self.rest.as_bytes().first().copied()
    }

    fn advance(&mut self) {
        self.rest = &self.rest[1..];
    }

    fn expression(&mut self, min_precedence: u8, depth: u8) -> Result<i64, &'static str> {
        if depth > 32 {
            return Err("expression nested too deeply");
        }
        let mut left = self.operand(depth)?;
        while let Some(op) = self.peek() {
            let precedence = match op {
                b'+' | b'-' => 1,
                b'*' | b'/' | b'%' => 2,
                _ => break,
            };
            if precedence < min_precedence {
                break;
            }
            self.advance();
            let right = self.expression(precedence + 1, depth + 1)?;
            left = match op {
                b'+' => left.checked_add(right),
                b'-' => left.checked_sub(right),
                b'*' => left.checked_mul(right),
                b'/' | b'%' if right == 0 => return Err("division by zero"),
                b'/' => left.checked_div(right),
                b'%' => left.checked_rem(right),
                _ => unreachable!(),
            }
            .ok_or("integer overflow")?;
        }
        Ok(left)
    }

    fn operand(&mut self, depth: u8) -> Result<i64, &'static str> {
        let sign = self.peek().filter(|byte| matches!(byte, b'+' | b'-'));
        if sign.is_some() {
            self.advance();
        }
        if self.peek() == Some(b'(') {
            self.advance();
            let value = self.expression(0, depth + 1)?;
            if self.peek() != Some(b')') {
                return Err("expected ')'");
            }
            self.advance();
            return if sign == Some(b'-') {
                value.checked_neg().ok_or("integer overflow")
            } else {
                Ok(value)
            };
        }

        let digits = self.rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return Err("expected an integer or '('");
        }
        let mut value = 0i64;
        for digit in self.rest.bytes().take(digits) {
            let digit = i64::from(digit - b'0');
            let signed = if sign == Some(b'-') { -digit } else { digit };
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(signed))
                .ok_or("integer overflow")?;
        }
        self.rest = &self.rest[digits..];
        Ok(value)
    }
}
