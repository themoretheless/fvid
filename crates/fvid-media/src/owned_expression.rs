//! Owned deterministic scalar expressions for media parameters and lookup tables.
//! Stateful/random/time-dependent AVExpr functions are not implemented here.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Expression(Node);
#[derive(Clone, Debug)]
enum Node {
    Number(f64),
    Variable(String),
    Unary(bool, Box<Node>),
    Binary(u8, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}
impl Node {
    fn depth(&self) -> usize {
        match self {
            Self::Number(_) | Self::Variable(_) => 1,
            Self::Unary(_, a) => 1 + a.depth(),
            Self::Binary(_, a, b) => 1 + a.depth().max(b.depth()),
            Self::Call(_, args) => 1 + args.iter().map(Self::depth).max().unwrap_or(0),
        }
    }
    fn validate_variables(&self, variables: &[(&str, f64)]) -> Result<()> {
        match self {
            Self::Variable(name) if !variables.iter().any(|(key, _)| *key == name) => {
                return Err(format!("unknown expression variable {name}"));
            }
            Self::Unary(_, a) => a.validate_variables(variables)?,
            Self::Binary(_, a, b) => {
                a.validate_variables(variables)?;
                b.validate_variables(variables)?;
            }
            Self::Call(_, args) => {
                for arg in args {
                    arg.validate_variables(variables)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
struct Parser<'a> {
    lut: bool,
    text: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn take(&mut self, token: u8) -> bool {
        self.space();
        if self.text.get(self.at) == Some(&token) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expression(&mut self, minimum: u8, depth: usize) -> Result<Node> {
        // Bound recursive call-stack use, not expression value or media memory.
        if depth > 128 {
            return Err("expression nesting exceeds call-stack bound".into());
        }
        self.space();
        // A signed dB literal converts the signed exponent before unary arithmetic.
        let negative_db = if self.text.get(self.at) == Some(&b'-') {
            let start = self.at;
            let mut end = start + 1;
            while self.text.get(end).is_some_and(|b| {
                b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')
            }) {
                end += 1;
            }
            if self.text.get(end..end + 2) == Some(b"dB") {
                std::str::from_utf8(&self.text[start..end])
                    .unwrap()
                    .parse::<f64>()
                    .ok()
                    .map(|value| (value, end + 2))
            } else {
                None
            }
        } else {
            None
        };
        let mut left = if let Some((value, end)) = negative_db {
            self.at = end;
            let mut value = 10f64.powf(value / 20.0);
            if self.text.get(self.at) == Some(&b'B') {
                value *= 8.0;
                self.at += 1;
            }
            Node::Number(value)
        } else if self.take(b'+') {
            Node::Unary(false, Box::new(self.expression(minimum.max(3), depth + 1)?))
        } else if self.take(b'-') {
            Node::Unary(true, Box::new(self.expression(minimum.max(3), depth + 1)?))
        } else if self.take(b'(') {
            let node = self.expression(0, depth + 1)?;
            if !self.take(b')') {
                return Err("missing expression closing parenthesis".into());
            }
            node
        } else {
            self.atom(depth + 1)?
        };
        loop {
            self.space();
            let operator = match self.text.get(self.at) {
                Some(b'+' | b'-' | b'*' | b'/' | b'^') => self.text[self.at],
                _ => break,
            };
            let precedence = match operator {
                b'+' | b'-' => 1,
                b'*' | b'/' => 2,
                _ => 3,
            };
            if precedence < minimum {
                break;
            }
            self.at += 1;
            let right = self.expression(precedence + 1, depth + 1)?;
            left = Node::Binary(operator, Box::new(left), Box::new(right));
            if left.depth() > 128 {
                return Err("expression tree exceeds call-stack bound".into());
            }
        }
        Ok(left)
    }
    fn atom(&mut self, depth: usize) -> Result<Node> {
        self.space();
        let start = self.at;
        if self
            .text
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            let mut number: f64;
            if self
                .text
                .get(start..start + 2)
                .is_some_and(|s| s == b"0x" || s == b"0X")
            {
                self.at += 2;
                let digits = self.at;
                let mut value = 0u64;
                while let Some(byte) = self
                    .text
                    .get(self.at)
                    .copied()
                    .filter(u8::is_ascii_hexdigit)
                {
                    let digit = (byte as char).to_digit(16).unwrap() as u64;
                    value = value.saturating_mul(16).saturating_add(digit);
                    self.at += 1;
                }
                if self.at == digits {
                    return Err("invalid hexadecimal expression number".into());
                }
                number = value as f64;
            } else {
                while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
                if self.text.get(self.at) == Some(&b'.') {
                    self.at += 1;
                    while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                        self.at += 1;
                    }
                }
                if self
                    .text
                    .get(self.at)
                    .is_some_and(|b| matches!(b, b'e' | b'E'))
                {
                    let exponent = self.at;
                    self.at += 1;
                    if self
                        .text
                        .get(self.at)
                        .is_some_and(|b| matches!(b, b'+' | b'-'))
                    {
                        self.at += 1;
                    }
                    let digits = self.at;
                    while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                        self.at += 1;
                    }
                    if self.at == digits {
                        self.at = exponent;
                    }
                }
                number = std::str::from_utf8(&self.text[start..self.at])
                    .unwrap()
                    .parse()
                    .map_err(|_| "invalid expression number")?;
            }
            if self.text.get(self.at..self.at + 2) == Some(b"dB") {
                number = 10f64.powf(number / 20.0);
                self.at += 2;
            } else {
                let exponent = match self.text.get(self.at) {
                    Some(b'y') => -24,
                    Some(b'z') => -21,
                    Some(b'a') => -18,
                    Some(b'f') => -15,
                    Some(b'p') => -12,
                    Some(b'n') => -9,
                    Some(b'u') => -6,
                    Some(b'm') => -3,
                    Some(b'c') => -2,
                    Some(b'd') => -1,
                    Some(b'h') => 2,
                    Some(b'k' | b'K') => 3,
                    Some(b'M') => 6,
                    Some(b'G') => 9,
                    Some(b'T') => 12,
                    Some(b'P') => 15,
                    Some(b'E') => 18,
                    Some(b'Z') => 21,
                    Some(b'Y') => 24,
                    _ => 0,
                };
                if exponent != 0 {
                    self.at += 1;
                    if self.text.get(self.at) == Some(&b'i') {
                        number *= 2f64.powf(exponent as f64 / 0.3);
                        self.at += 1;
                    } else {
                        number *= 10f64.powi(exponent);
                    }
                }
            }
            if self.text.get(self.at) == Some(&b'B') {
                number *= 8.0;
                self.at += 1;
            }
            return Ok(Node::Number(number));
        }
        while self
            .text
            .get(self.at)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.at += 1;
        }
        if self.at == start {
            return Err("expected expression value".into());
        }
        let name = std::str::from_utf8(&self.text[start..self.at]).unwrap();
        if self.take(b'(') {
            let arities = match name {
                "if" | "ifnot" => (2, 3),
                "clip" if self.lut => (1, 3),
                "gammaval" | "gammaval709" if self.lut => (1, 1),
                "clip" | "between" | "lerp" => (3, 3),
                "atan2" | "hypot" | "pow" | "min" | "max" | "mod" | "eq" | "gt" | "gte" | "lt"
                | "lte" | "bitand" | "bitor" => (2, 2),
                "abs" | "acos" | "asin" | "atan" | "cos" | "sin" | "tan" | "cosh" | "sinh"
                | "tanh" | "exp" | "log" | "sqrt" | "ceil" | "floor" | "trunc" | "round"
                | "not" | "sgn" | "isnan" | "isinf" | "squish" | "gauss" => (1, 1),
                _ => return Err(format!("unsupported expression function {name}")),
            };
            let mut args = Vec::new();
            if !self.take(b')') {
                loop {
                    args.try_reserve(1).map_err(|e| e.to_string())?;
                    args.push(self.expression(0, depth + 1)?);
                    if self.take(b')') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err("expected expression argument separator".into());
                    }
                }
            }
            if args.len() < arities.0
                || args.len() > arities.1
                || (name == "clip" && args.len() == 2)
            {
                return Err("invalid expression function arity".into());
            }
            Ok(Node::Call(name.to_owned(), args))
        } else {
            Ok(match name {
                "PI" => Node::Number(std::f64::consts::PI),
                "E" => Node::Number(std::f64::consts::E),
                "PHI" => Node::Number(1.618033988749895),
                "NAN" | "nan" => Node::Number(f64::NAN),
                "INF" | "inf" => Node::Number(f64::INFINITY),
                _ => Node::Variable(name.to_owned()),
            })
        }
    }
}
impl Expression {
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_mode(text, false)
    }
    pub(crate) fn parse_lut(text: &str) -> Result<Self> {
        Self::parse_mode(text, true)
    }
    fn parse_mode(text: &str, lut: bool) -> Result<Self> {
        let mut parser = Parser {
            lut,
            text: text.as_bytes(),
            at: 0,
        };
        let node = parser.expression(0, 0)?;
        parser.space();
        if parser.at != parser.text.len() {
            return Err("trailing expression tokens".into());
        }
        if node.depth() > 128 {
            return Err("expression tree exceeds call-stack bound".into());
        }
        Ok(Self(node))
    }
    pub fn evaluate(&self, variables: &[(&str, f64)]) -> Result<f64> {
        self.0.validate_variables(variables)?;
        evaluate(&self.0, variables)
    }
}
pub fn constant(text: &str) -> Result<f64> {
    Expression::parse(text)?.evaluate(&[])
}
fn evaluate(node: &Node, vars: &[(&str, f64)]) -> Result<f64> {
    let number = match node {
        Node::Number(number) => *number,
        Node::Variable(name) => vars
            .iter()
            .find_map(|(key, value)| (*key == name).then_some(*value))
            .ok_or_else(|| format!("unknown expression variable {name}"))?,
        Node::Unary(negative, node) => {
            let value = evaluate(node, vars)?;
            if *negative { -value } else { value }
        }
        Node::Binary(operator, a, b) => {
            let a = evaluate(a, vars)?;
            let b = evaluate(b, vars)?;
            match operator {
                b'+' => a + b,
                b'-' => a - b,
                b'*' => a * b,
                b'/' => a / b,
                _ => a.powf(b),
            }
        }
        Node::Call(name, args) => {
            let a = evaluate(&args[0], vars)?;
            if name == "if" || name == "ifnot" {
                let selected = if (a != 0.0) == (name == "if") {
                    Some(1)
                } else if args.len() == 3 {
                    Some(2)
                } else {
                    None
                };
                return selected.map_or(Ok(0.0), |index| evaluate(&args[index], vars));
            }
            let b = if args.len() > 1 {
                evaluate(&args[1], vars)?
            } else {
                0.0
            };
            let c = if args.len() > 2 {
                evaluate(&args[2], vars)?
            } else {
                0.0
            };
            let boolean = |condition| if condition { 1.0 } else { 0.0 };
            match name.as_str() {
                "abs" => a.abs(),
                "acos" => a.acos(),
                "asin" => a.asin(),
                "atan" => a.atan(),
                "atan2" => a.atan2(b),
                "cos" => a.cos(),
                "sin" => a.sin(),
                "tan" => a.tan(),
                "cosh" => a.cosh(),
                "sinh" => a.sinh(),
                "tanh" => a.tanh(),
                "exp" => a.exp(),
                "log" => a.ln(),
                "sqrt" => a.sqrt(),
                "ceil" => a.ceil(),
                "floor" => a.floor(),
                "trunc" => a.trunc(),
                "round" => a.round(),
                "hypot" => a.hypot(b),
                "pow" => a.powf(b),
                "min" => {
                    if a < b {
                        a
                    } else {
                        b
                    }
                }
                "max" => {
                    if a > b {
                        a
                    } else {
                        b
                    }
                }
                "mod" => a - b * (a / b).floor(),
                "eq" => boolean(a == b),
                "gt" => boolean(a > b),
                "gte" => boolean(a >= b),
                "lt" => boolean(a < b),
                "lte" => boolean(a <= b),
                "not" => boolean(a == 0.0),
                "sgn" => {
                    if a > 0.0 {
                        1.0
                    } else if a < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                "isnan" => boolean(a.is_nan()),
                "isinf" => boolean(a.is_infinite()),
                "between" => boolean(a >= b && a <= c),
                "lerp" => a + (b - a) * c,
                "gammaval" | "gammaval709" => {
                    let context = |name| {
                        vars.iter()
                            .find_map(|(key, value)| (*key == name).then_some(*value))
                            .ok_or_else(|| format!("missing LUT context {name}"))
                    };
                    let minimum = context("minval")?;
                    let maximum = context("maxval")?;
                    let level = (context("clipval")? - minimum) / (maximum - minimum);
                    let adjusted = if name == "gammaval" {
                        level.powf(a)
                    } else if level < 0.018 {
                        4.5 * level
                    } else {
                        1.099 * level.powf(1.0 / a) - 0.099
                    };
                    adjusted * (maximum - minimum) + minimum
                }
                "clip" if args.len() == 1 => {
                    let context = |name| {
                        vars.iter()
                            .find_map(|(key, value)| (*key == name).then_some(*value))
                            .ok_or_else(|| format!("missing LUT context {name}"))
                    };
                    let minimum = context("minval")?;
                    let maximum = context("maxval")?;
                    if minimum > maximum || minimum.is_nan() || maximum.is_nan() {
                        return Err("invalid LUT range".into());
                    }
                    a.clamp(minimum, maximum)
                }
                "clip" => {
                    if b > c || b.is_nan() || c.is_nan() {
                        return Err("invalid expression clip interval".into());
                    }
                    a.clamp(b, c)
                }
                "bitand" => {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        ((a as i64) & (b as i64)) as f64
                    }
                }
                "bitor" => {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        ((a as i64) | (b as i64)) as f64
                    }
                }
                "squish" => 1.0 / (1.0 + (4.0 * a).exp()),
                "gauss" => (-a * a / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt(),
                _ => return Err("unsupported expression function".into()),
            }
        }
    };
    Ok(number)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_precedence_constants_functions_and_lazy_branches() {
        for (expression, expected) in [
            ("1+2*3", 7.0),
            ("(1+2)*3", 9.0),
            ("2^3^2", 64.0),
            ("-2^2", -4.0),
            ("2^-3^2", 0.015625),
            ("1e-2+0.5", 0.51),
            ("if(1,2,clip(0,3,2))", 2.0),
            ("ifnot(1,clip(0,3,2),3)", 3.0),
            ("if(0,clip(0,3,2))", 0.0),
            ("max(2,min(4,3))", 3.0),
            ("sin(PI/2)", 1.0),
            ("mod(-3,2)", 1.0),
            ("clip(5,1,3)", 3.0),
        ] {
            assert!(
                (constant(expression).unwrap() - expected).abs() < 1e-12,
                "{expression}"
            );
        }
        let expression = Expression::parse("val*0.8+w/h").unwrap();
        assert_eq!(
            expression
                .evaluate(&[("val", 100.0), ("w", 4.0), ("h", 2.0)])
                .unwrap(),
            82.0
        );
    }
    #[test]
    fn numeric_hex_suffixes_and_exponent_ambiguity() {
        for (text, expected) in [
            ("0xFF", 255.),
            ("0X10", 16.),
            ("0xffffffffffffffffffff", u64::MAX as f64),
            ("1k", 1000.),
            ("1Ki", 1024.),
            ("1KiB", 8192.),
            ("1B", 8.),
            ("20dB", 10.),
            ("-20dB", 0.1),
            ("1E", 1e18),
            ("1E-2", 0.01),
            ("1m", 0.001),
            ("0x10Ki", 16384.),
            ("1.5k", 1500.),
            ("1e2k", 100000.),
        ] {
            let actual = constant(text).unwrap();
            assert!(
                (actual - expected).abs() <= expected.abs() * 1e-14,
                "{text}: {actual}"
            );
        }
        for text in ["0x", "0Xz", "1e", "1e+", "1..2", "1Kii", "1BB", "1dBBx"] {
            assert!(constant(text).is_err(), "{text}");
        }
    }
    #[test]
    fn nonfinite_values_propagate_instead_of_becoming_valid_filter_parameters() {
        for text in [
            "sqrt(-1)",
            "bitand(NAN,1)",
            "bitor(1,NAN)",
            "min(1,NAN)",
            "max(1,NAN)",
        ] {
            assert!(constant(text).unwrap().is_nan(), "{text}");
        }
        assert!(!constant("1/0").unwrap().is_finite());
    }
    #[test]
    fn malformed_unsupported_and_excessive_nesting_refuse() {
        for text in [
            "",
            "1 2",
            "1+",
            "(1",
            "sin()",
            "pow(2)",
            "random(0)",
            "clip(1,3,2)",
            "unknown",
        ] {
            assert!(constant(text).is_err(), "{text}");
        }
        assert!(
            constant(&format!("{}1{}", "(".repeat(200), ")".repeat(200)))
                .unwrap_err()
                .contains("nesting")
        );
        assert!(
            constant(&std::iter::repeat_n("1", 200).collect::<Vec<_>>().join("+"))
                .unwrap_err()
                .contains("call-stack")
        );
        assert!(constant("if(1,2,unknown)").is_err());
    }
}

#[cfg(test)]
mod lut_context_tests {
    use super::*;
    #[test]
    fn lut_functions_use_their_component_range() {
        let vars = |value| {
            [
                ("minval", 16.0),
                ("maxval", 235.0),
                ("clipval", value),
                ("val", value),
            ]
        };
        assert_eq!(
            Expression::parse_lut("clip(-100)")
                .unwrap()
                .evaluate(&vars(16.0))
                .unwrap(),
            16.0
        );
        assert_eq!(
            Expression::parse_lut("clip(1000)")
                .unwrap()
                .evaluate(&vars(235.0))
                .unwrap(),
            235.0
        );
        let square = Expression::parse_lut("gammaval(2)").unwrap();
        assert_eq!(square.evaluate(&vars(16.0)).unwrap(), 16.0);
        assert_eq!(square.evaluate(&vars(235.0)).unwrap(), 235.0);
        assert_eq!(square.evaluate(&vars(125.5)).unwrap(), 70.75);
        let rec709 = Expression::parse_lut("gammaval709(2)").unwrap();
        assert!((rec709.evaluate(&vars(18.19)).unwrap() - 25.855).abs() < 1e-10);
        assert!((rec709.evaluate(&vars(70.75)).unwrap() - 114.6595).abs() < 1e-10);
        for text in ["clip(val)", "gammaval(2)", "gammaval709(2)"] {
            assert!(Expression::parse(text).is_err());
        }
        assert_eq!(
            Expression::parse_lut("clip(3,0,2)")
                .unwrap()
                .evaluate(&[])
                .unwrap(),
            2.0
        );
        assert!(Expression::parse_lut("clip(1,2)").is_err());
        assert!(rec709.evaluate(&[]).is_err());
    }
}
