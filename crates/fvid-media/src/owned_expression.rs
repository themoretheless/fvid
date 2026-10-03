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
                return Err(format!("unknown expression variable {name}"))
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
        let mut left = if self.take(b'+') {
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
            while self
                .text
                .get(self.at)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
            {
                self.at += 1;
            }
            if self
                .text
                .get(self.at)
                .is_some_and(|b| matches!(*b, b'e' | b'E'))
            {
                self.at += 1;
                if self
                    .text
                    .get(self.at)
                    .is_some_and(|b| matches!(*b, b'+' | b'-'))
                {
                    self.at += 1;
                }
                while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
            }
            let number = std::str::from_utf8(&self.text[start..self.at])
                .unwrap()
                .parse()
                .map_err(|_| "invalid expression number")?;
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
            if args.len() < arities.0 || args.len() > arities.1 {
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
        let mut parser = Parser {
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
            if *negative {
                -value
            } else {
                value
            }
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
