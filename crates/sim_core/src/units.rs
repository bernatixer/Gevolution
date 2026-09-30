//! Physical units: SI base-dimension exponents plus an absolute-temperature flag.
//!
//! Only canonical SI scale is supported, so no numeric conversion is ever needed.
//! Absolute temperature (`K`) is affine: it can be offset by an interval (`dK`)
//! and two absolutes subtract to an interval, but absolutes never multiply.

use std::fmt;

/// Exponents of kg, m, s, K. `absolute` marks an affine absolute temperature.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Unit {
    pub kg: i8,
    pub m: i8,
    pub s: i8,
    pub k: i8,
    pub absolute: bool,
}

pub const DIMENSIONLESS: Unit = Unit {
    kg: 0,
    m: 0,
    s: 0,
    k: 0,
    absolute: false,
};

impl Unit {
    pub const fn new(kg: i8, m: i8, s: i8, k: i8) -> Unit {
        Unit {
            kg,
            m,
            s,
            k,
            absolute: false,
        }
    }
    pub const fn absolute_temperature() -> Unit {
        Unit {
            kg: 0,
            m: 0,
            s: 0,
            k: 1,
            absolute: true,
        }
    }
    pub fn is_dimensionless(&self) -> bool {
        *self == DIMENSIONLESS
    }
    /// The interval unit corresponding to this unit (drops the absolute flag).
    pub fn interval(&self) -> Unit {
        Unit { absolute: false, ..*self }
    }
    pub fn mul(&self, o: &Unit) -> Result<Unit, String> {
        if self.absolute || o.absolute {
            return Err("absolute temperature cannot be multiplied or divided; subtract a reference temperature first".into());
        }
        Ok(Unit::new(self.kg + o.kg, self.m + o.m, self.s + o.s, self.k + o.k))
    }
    pub fn div(&self, o: &Unit) -> Result<Unit, String> {
        if self.absolute || o.absolute {
            return Err("absolute temperature cannot be multiplied or divided; subtract a reference temperature first".into());
        }
        Ok(Unit::new(self.kg - o.kg, self.m - o.m, self.s - o.s, self.k - o.k))
    }
    pub fn powi(&self, n: i8) -> Result<Unit, String> {
        if self.absolute {
            return Err("absolute temperature cannot be raised to a power".into());
        }
        Ok(Unit::new(self.kg * n, self.m * n, self.s * n, self.k * n))
    }
    /// Result of `a + b`.
    pub fn add(&self, o: &Unit) -> Result<Unit, String> {
        match (self.absolute, o.absolute) {
            (false, false) if self == o => Ok(*self),
            (true, false) | (false, true) if self.interval() == o.interval() => Ok(Unit::absolute_temperature()),
            (true, true) => Err("cannot add two absolute temperatures".into()),
            _ => Err(format!("cannot add {} and {}", self, o)),
        }
    }
    /// Result of `a - b`.
    pub fn sub(&self, o: &Unit) -> Result<Unit, String> {
        match (self.absolute, o.absolute) {
            (false, false) if self == o => Ok(*self),
            (true, true) => Ok(self.interval()),
            (true, false) if self.interval() == *o => Ok(*self),
            (false, true) => Err("cannot subtract an absolute temperature from an interval".into()),
            _ => Err(format!("cannot subtract {} from {}", o, self)),
        }
    }
    /// Units that can be compared, min/maxed, selected between, or interpolated.
    pub fn same(&self, o: &Unit) -> bool {
        self == o
    }

    pub fn parse(s: &str) -> Result<Unit, String> {
        let toks = tokenize(s)?;
        let mut p = Parser { toks, pos: 0 };
        let u = p.expr()?;
        if p.pos != p.toks.len() {
            return Err(format!("unexpected trailing input in unit '{s}'"));
        }
        Ok(u)
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.absolute {
            return write!(f, "K");
        }
        let parts = [("kg", self.kg), ("m", self.m), ("s", self.s), ("dK", self.k)];
        let num: Vec<String> = parts
            .iter()
            .filter(|(_, e)| *e > 0)
            .map(|(n, e)| if *e == 1 { n.to_string() } else { format!("{n}^{e}") })
            .collect();
        let den: Vec<String> = parts
            .iter()
            .filter(|(_, e)| *e < 0)
            .map(|(n, e)| if *e == -1 { n.to_string() } else { format!("{n}^{}", -e) })
            .collect();
        let n = if num.is_empty() { "1".to_string() } else { num.join("*") };
        match den.len() {
            0 => write!(f, "{n}"),
            1 => write!(f, "{n}/{}", den[0]),
            _ => write!(f, "{n}/({})", den.join("*")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Sym(String),
    Int(i8),
    Mul,
    Div,
    Pow,
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' => i += 1,
            '*' | '.' => {
                out.push(Tok::Mul);
                i += 1
            }
            '/' => {
                out.push(Tok::Div);
                i += 1
            }
            '^' => {
                out.push(Tok::Pow);
                i += 1
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1
            }
            '-' | '0'..='9' => {
                let st = i;
                i += 1;
                while i < cs.len() && cs[i].is_ascii_digit() {
                    i += 1;
                }
                let t: String = cs[st..i].iter().collect();
                out.push(Tok::Int(t.parse().map_err(|_| format!("bad exponent '{t}'"))?));
            }
            c if c.is_ascii_alphabetic() => {
                let st = i;
                while i < cs.len() && cs[i].is_ascii_alphabetic() {
                    i += 1;
                }
                out.push(Tok::Sym(cs[st..i].iter().collect()));
            }
            _ => return Err(format!("unexpected character '{c}' in unit '{s}'")),
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn expr(&mut self) -> Result<Unit, String> {
        let mut u = self.term()?;
        loop {
            match self.toks.get(self.pos) {
                Some(Tok::Mul) => {
                    self.pos += 1;
                    u = u.mul(&self.term()?)?
                }
                Some(Tok::Div) => {
                    self.pos += 1;
                    u = u.div(&self.term()?)?
                }
                _ => return Ok(u),
            }
        }
    }
    fn term(&mut self) -> Result<Unit, String> {
        let base = self.factor()?;
        if let Some(Tok::Pow) = self.toks.get(self.pos) {
            self.pos += 1;
            match self.toks.get(self.pos) {
                Some(Tok::Int(n)) => {
                    let n = *n;
                    self.pos += 1;
                    return base.powi(n);
                }
                _ => return Err("expected integer exponent after '^'".into()),
            }
        }
        Ok(base)
    }
    fn factor(&mut self) -> Result<Unit, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::LParen) => {
                self.pos += 1;
                let u = self.expr()?;
                if self.toks.get(self.pos) != Some(&Tok::RParen) {
                    return Err("expected ')' in unit".into());
                }
                self.pos += 1;
                Ok(u)
            }
            Some(Tok::Int(1)) => {
                self.pos += 1;
                Ok(DIMENSIONLESS)
            }
            Some(Tok::Sym(s)) => {
                self.pos += 1;
                symbol(&s)
            }
            other => Err(format!("unexpected token {other:?} in unit")),
        }
    }
}

fn symbol(s: &str) -> Result<Unit, String> {
    Ok(match s {
        "kg" => Unit::new(1, 0, 0, 0),
        "m" => Unit::new(0, 1, 0, 0),
        "s" => Unit::new(0, 0, 1, 0),
        "K" => Unit::absolute_temperature(),
        "dK" => Unit::new(0, 0, 0, 1),
        "J" => Unit::new(1, 2, -2, 0),
        "W" => Unit::new(1, 2, -3, 0),
        "rad" => DIMENSIONLESS,
        _ => {
            return Err(format!("unknown unit symbol '{s}' (supported: kg, m, s, K, dK, J, W, rad, 1)"));
        }
    })
}

impl serde::Serialize for Unit {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_algebra() {
        let rate = Unit::parse("kg/s").unwrap();
        let flux = Unit::parse("kg/(m^2*s)").unwrap();
        let area = Unit::parse("m^2").unwrap();
        assert_eq!(flux.mul(&area).unwrap(), rate);
        assert_eq!(rate.mul(&Unit::parse("s").unwrap()).unwrap(), Unit::parse("kg").unwrap());
        assert!(Unit::parse("kg").unwrap().add(&Unit::parse("K").unwrap()).is_err());
        let t = Unit::parse("K").unwrap();
        assert_eq!(t.sub(&t).unwrap(), Unit::parse("dK").unwrap());
        assert_eq!(t.add(&Unit::parse("dK").unwrap()).unwrap(), t);
        assert!(t.mul(&DIMENSIONLESS).is_err());
        assert!(t.add(&t).is_err());
        assert_eq!(Unit::parse("1/s").unwrap().to_string(), "1/s");
        assert_eq!(Unit::parse("J").unwrap().to_string(), "kg*m^2/s^2");
        assert!(Unit::parse("furlong").is_err());
    }
}
