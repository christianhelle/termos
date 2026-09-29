//! Interactive prompt session for running many commands and queries.

use std::time::Duration;

/// A line typed at the interactive prompt.
#[derive(Debug, PartialEq)]
pub enum Input {
    /// Nothing but whitespace.
    Empty,
    /// SQL to run against the current container.
    Query(String),
    /// A slash command split into words, without the leading slash.
    Slash(Vec<String>),
}

/// Works out what a line typed at the prompt asks for.
pub fn parse_input(line: &str) -> anyhow::Result<Input> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Input::Empty);
    }
    match line.strip_prefix('/') {
        Some(command) => Ok(Input::Slash(
            command.split_whitespace().map(String::from).collect(),
        )),
        None => Ok(Input::Query(line.to_string())),
    }
}

/// Formats how long something took, such as "245 ms" or "1.23 s".
pub fn format_elapsed(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.2} s", elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Input {
        Input::Slash(words.iter().map(|w| w.to_string()).collect())
    }

    #[test]
    fn blank_line_is_empty() {
        assert_eq!(parse_input("   \n").unwrap(), Input::Empty);
    }

    #[test]
    fn text_without_a_slash_is_a_trimmed_query() {
        assert_eq!(
            parse_input("  SELECT * FROM c \n").unwrap(),
            Input::Query("SELECT * FROM c".into())
        );
    }

    #[test]
    fn text_with_a_leading_slash_is_a_command_split_into_words() {
        assert_eq!(
            parse_input(" /items get  --id c-1\n").unwrap(),
            words(&["items", "get", "--id", "c-1"])
        );
    }

    #[test]
    fn elapsed_under_a_second_is_shown_in_milliseconds() {
        assert_eq!(format_elapsed(Duration::from_micros(245_900)), "245 ms");
        assert_eq!(format_elapsed(Duration::ZERO), "0 ms");
    }

    #[test]
    fn elapsed_of_a_second_or_more_is_shown_in_seconds() {
        assert_eq!(format_elapsed(Duration::from_millis(1_000)), "1.00 s");
        assert_eq!(format_elapsed(Duration::from_millis(12_346)), "12.35 s");
    }
}
