use crate::pipeline::{Lexer, Token};

pub struct GoTestLexer;

impl Lexer for GoTestLexer {
    fn lex(&self, input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();

        for line in input.lines() {
            let trimmed = line.trim();

            if let Some(test_name) = parse_go_test_failure_header(trimmed) {
                tokens.push(Token::FailureHeader { test_name });
                continue;
            }

            if let Some((file, line_num, message)) = parse_go_test_location(trimmed) {
                tokens.push(Token::PanicLocation {
                    file,
                    line: line_num,
                    column: 1,
                });

                if let Some((expected, actual)) = parse_expected_actual(&message) {
                    tokens.push(Token::AssertionRight { value: expected });
                    tokens.push(Token::AssertionLeft { value: actual });
                }

                tokens.push(Token::PanicMessage { message });
                continue;
            }

            if let Some(message) = parse_go_test_message(trimmed) {
                tokens.push(Token::PanicMessage { message });
            }
        }

        tokens
    }
}

fn parse_go_test_failure_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("--- FAIL: ")?;
    let (name, _) = rest.split_once(" (").unwrap_or((rest, ""));
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn parse_go_test_location(line: &str) -> Option<(String, u32, String)> {
    let mut parts = line.splitn(3, ':');
    let file = parts.next()?.trim();
    let line_num = parts.next()?.trim().parse::<u32>().ok()?;
    let message = parts.next()?.trim();

    if file.is_empty() || message.is_empty() {
        return None;
    }

    Some((file.to_string(), line_num, message.to_string()))
}

fn parse_expected_actual(message: &str) -> Option<(String, String)> {
    if let Some(expected_part) = message.strip_prefix("expected ") {
        let (expected, actual_part) = expected_part.split_once(", got ")?;
        let expected = expected.trim();
        let actual = actual_part.trim();
        if expected.is_empty() || actual.is_empty() {
            return None;
        }
        return Some((expected.to_string(), actual.to_string()));
    }

    if let Some(actual_part) = message.strip_prefix("got ") {
        let (actual, expected_part) = actual_part.split_once(", want ")?;
        let actual = actual.trim();
        let expected = expected_part.trim();
        if expected.is_empty() || actual.is_empty() {
            return None;
        }
        return Some((expected.to_string(), actual.to_string()));
    }

    None
}

fn parse_go_test_message(line: &str) -> Option<String> {
    let message = line.strip_prefix("panic:")?.trim();
    if message.is_empty() {
        return None;
    }
    Some(format!("panic: {message}"))
}

#[cfg(test)]
mod tests {
    use super::GoTestLexer;
    use crate::pipeline::{Lexer, Token};

    #[test]
    fn lexes_go_test_assertion_failure() {
        let input = "\
--- FAIL: TestValidateToken (0.00s)
    auth_test.go:42: expected 401, got 403
FAIL
FAIL    github.com/acme/auth  0.123s";

        let tokens = GoTestLexer.lex(input);

        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::FailureHeader { test_name } if test_name == "TestValidateToken"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "auth_test.go" && *line == 42 && *column == 1
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionRight { value } if value == "401"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionLeft { value } if value == "403"
        )));
    }
}
