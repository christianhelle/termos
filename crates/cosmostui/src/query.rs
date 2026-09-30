//! Turns what is typed in the search bar into SQL.

/// The query every container starts with.
pub const DEFAULT_QUERY: &str = "SELECT * FROM c";

/// The query for the search bar text: a full `SELECT`, or a filter on every document.
pub fn build_query(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return DEFAULT_QUERY.to_string();
    }
    text.to_string()
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
}
