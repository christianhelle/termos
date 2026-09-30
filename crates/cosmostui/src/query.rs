//! Turns what is typed in the search bar into SQL.

/// The query every container starts with.
pub const DEFAULT_QUERY: &str = "SELECT * FROM c";

/// The query for the search bar text: a full `SELECT`, or a filter on every document.
pub fn build_query(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return DEFAULT_QUERY.to_string();
    }
    if starts_with_keyword(text, "SELECT") {
        return text.to_string();
    }
    format!("{DEFAULT_QUERY} {text}")
}

/// Whether the text starts with the keyword as a whole word, ignoring case.
fn starts_with_keyword(text: &str, keyword: &str) -> bool {
    let word = text.split_whitespace().next().unwrap_or_default();
    word.eq_ignore_ascii_case(keyword)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_search_selects_every_document() {
        assert_eq!(build_query(""), "SELECT * FROM c");
        assert_eq!(build_query("   "), "SELECT * FROM c");
    }

    #[test]
    fn a_select_statement_runs_as_typed() {
        assert_eq!(
            build_query(" select c.id FROM c WHERE c.total > 10 "),
            "select c.id FROM c WHERE c.total > 10"
        );
    }

    #[test]
    fn clauses_filter_every_document() {
        assert_eq!(
            build_query("WHERE c.status = 'open'"),
            "SELECT * FROM c WHERE c.status = 'open'"
        );
        assert_eq!(
            build_query("order by c._ts DESC"),
            "SELECT * FROM c order by c._ts DESC"
        );
    }
}
