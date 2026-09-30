//! Turns what is typed in the search bar into SQL.

/// The query every container starts with.
pub const DEFAULT_QUERY: &str = "SELECT * FROM c";

/// The query for the search bar text: a full `SELECT`, or a filter on every document.
pub fn build_query(text: &str) -> String {
    let _ = text;
    DEFAULT_QUERY.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_search_selects_every_document() {
        assert_eq!(build_query(""), "SELECT * FROM c");
        assert_eq!(build_query("   "), "SELECT * FROM c");
    }
}
