use crate::pipeline::{Lexer, Token};

pub struct JestLexer;

impl Lexer for JestLexer {
    fn lex(&self, input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();

        for line in input.lines() {
            let trimmed = line.trim();

            if let Some(test_name) = parse_jest_failure_header(trimmed) {
                tokens.push(Token::FailureHeader { test_name });
                continue;
            }

            if let Some((file, line_num, column_num)) = parse_jest_location(trimmed) {
                tokens.push(Token::PanicLocation {
                    file,
                    line: line_num,
                    column: column_num,
                });
                continue;
            }

            if let Some(expected) = trimmed.strip_prefix("Expected:") {
                let expected = expected.trim();
                if !expected.is_empty() {
                    tokens.push(Token::AssertionRight {
                        value: expected.to_string(),
                    });
                }
                continue;
            }

            if let Some(received) = trimmed.strip_prefix("Received:") {
                let received = received.trim();
                if !received.is_empty() {
                    tokens.push(Token::AssertionLeft {
                        value: received.to_string(),
                    });
                }
                continue;
            }

            if is_jest_message_line(trimmed) {
                tokens.push(Token::PanicMessage {
                    message: trimmed.to_string(),
                });
            }
        }

        tokens
    }
}

fn parse_jest_failure_header(line: &str) -> Option<String> {
    let test_name = line.strip_prefix("● ")?;
    let test_name = test_name.trim();
    if test_name.is_empty() {
        return None;
    }
    Some(test_name.to_string())
}

fn parse_jest_location(line: &str) -> Option<(String, u32, u32)> {
    if let Some((_, body)) = line.split_once('(') {
        let location = body.trim_end_matches(')').trim();
        if let Some(location) = parse_file_line_column(location) {
            return Some(location);
        }
    }

    if let Some(location) = line.strip_prefix("at ") {
        return parse_file_line_column(location.trim());
    }

    None
}

fn parse_file_line_column(location: &str) -> Option<(String, u32, u32)> {
    let mut parts = location.rsplitn(3, ':');
    let col = parts.next()?.parse::<u32>().ok()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let file = parts.next()?.trim();
    if file.is_empty() {
        return None;
    }
    Some((file.to_string(), line, col))
}

fn is_jest_message_line(line: &str) -> bool {
    line.starts_with("expect(") || line.starts_with("Expected:") || line.starts_with("Received:")
}

#[cfg(test)]
mod tests {
    use super::JestLexer;
    use crate::pipeline::{Lexer, Token};

    #[test]
    fn lexes_jest_assertion_failure() {
        let input = "\
FAIL src/auth.test.ts
  auth
    x rejects expired token (5 ms)

  ● auth > rejects expired token

    expect(received).toBe(expected) // Object.is equality

    Expected: 401
    Received: 403

      at Object.<anonymous> (src/auth.test.ts:14:23)";

        let tokens = JestLexer.lex(input);

        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::FailureHeader { test_name } if test_name == "auth > rejects expired token"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "src/auth.test.ts" && *line == 14 && *column == 23
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionLeft { value } if value == "403"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionRight { value } if value == "401"
        )));
    }
}
