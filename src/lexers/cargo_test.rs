use crate::pipeline::{Lexer, Token};

pub struct CargoTestLexer;

impl Lexer for CargoTestLexer {
    fn lex(&self, input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();

        for line in input.lines() {
            let trimmed = line.trim();

            if let Some(test_name) = parse_failure_header(trimmed) {
                tokens.push(Token::FailureHeader { test_name });
                continue;
            }

            if let Some(test_name) = parse_panic_thread_name(trimmed) {
                tokens.push(Token::PanicThread { test_name });
            }

            if let Some(details) = parse_panic_details(trimmed) {
                if let Some((file, line_num, col_num)) = details.location {
                    tokens.push(Token::PanicLocation {
                        file,
                        line: line_num,
                        column: col_num,
                    });
                }
                if let Some(message) = details.message {
                    tokens.push(Token::PanicMessage { message });
                }
                continue;
            }

            if let Some(value) = parse_assertion_side(trimmed, "left:") {
                tokens.push(Token::AssertionLeft { value });
                continue;
            }

            if let Some(value) = parse_assertion_side(trimmed, "right:") {
                tokens.push(Token::AssertionRight { value });
                continue;
            }

            if trimmed.contains("assertion") && trimmed.contains("failed") {
                tokens.push(Token::PanicMessage {
                    message: trimmed.to_string(),
                });
            }
        }

        tokens
    }
}

fn parse_failure_header(line: &str) -> Option<String> {
    if line.starts_with("---- ") && line.ends_with(" stdout ----") {
        let body = line
            .trim_start_matches("---- ")
            .trim_end_matches(" stdout ----")
            .trim();
        if !body.is_empty() {
            return Some(body.to_string());
        }
    }
    None
}

fn parse_assertion_side(line: &str, side: &str) -> Option<String> {
    line.strip_prefix(side)
        .map(|value| value.trim().to_string())
}

fn parse_panic_thread_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("thread '")?;
    let (test_name, _) = rest.split_once("' panicked at ")?;
    let test_name = test_name.trim();
    if test_name.is_empty() {
        return None;
    }
    Some(test_name.to_string())
}

struct PanicDetails {
    location: Option<(String, u32, u32)>,
    message: Option<String>,
}

fn parse_panic_details(line: &str) -> Option<PanicDetails> {
    let (_, payload) = line.split_once(" panicked at ")?;
    let payload = payload.trim();

    if let Some((message_part, location_part)) = payload.rsplit_once(", ") {
        let location_part = location_part.trim_end_matches(':').trim();
        if let Some(location) = parse_file_line_column(location_part) {
            return Some(PanicDetails {
                location: Some(location),
                message: normalize_panic_message(message_part),
            });
        }
    }

    if let Some((location_part, message_part)) = payload.split_once(": ") {
        if let Some(location) = parse_file_line_column(location_part.trim()) {
            return Some(PanicDetails {
                location: Some(location),
                message: Some(message_part.trim().to_string()),
            });
        }
    }

    let location = parse_file_line_column(payload.trim_end_matches(':').trim());
    Some(PanicDetails {
        location,
        message: None,
    })
}

fn normalize_panic_message(message_part: &str) -> Option<String> {
    let message = message_part
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim();
    if message.is_empty() {
        None
    } else {
        Some(message.to_string())
    }
}

fn parse_file_line_column(location: &str) -> Option<(String, u32, u32)> {
    let mut parts = location.rsplitn(3, ':');
    let col = parts.next()?.parse::<u32>().ok()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let file = parts.next()?.to_string();
    if file.is_empty() {
        return None;
    }
    Some((file, line, col))
}

#[cfg(test)]
mod tests {
    use super::CargoTestLexer;
    use crate::pipeline::{Lexer, Token};

    #[test]
    fn lexes_basic_cargo_test_failure() {
        let input = "\
---- tests::auth_invalid_token stdout ----
thread 'tests::auth_invalid_token' panicked at src/auth.rs:89:9:
assertion `left == right` failed
  left: 403
 right: 401";

        let tokens = CargoTestLexer.lex(input);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::FailureHeader { test_name } if test_name == "tests::auth_invalid_token"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "src/auth.rs" && *line == 89 && *column == 9
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicThread { test_name } if test_name == "tests::auth_invalid_token"
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

    #[test]
    fn lexes_quoted_panic_message_format() {
        let input = "\
---- tests::auth_invalid_token stdout ----
thread 'tests::auth_invalid_token' panicked at 'assertion `left == right` failed', src/auth.rs:89:9
  left: 403
 right: 401";

        let tokens = CargoTestLexer.lex(input);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "src/auth.rs" && *line == 89 && *column == 9
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicMessage { message } if message == "assertion `left == right` failed"
        )));
    }

    #[test]
    fn lexes_inline_location_then_message_format() {
        let input = "\
---- tests::panic_case stdout ----
thread 'tests::panic_case' panicked at src/main.rs:12:5: called `Option::unwrap()` on a `None` value";

        let tokens = CargoTestLexer.lex(input);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "src/main.rs" && *line == 12 && *column == 5
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicMessage { message } if message.contains("None")
        )));
    }
}
