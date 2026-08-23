use zerobeat_catalog::{AudioQuality, CatalogError, SearchRequest};

#[test]
fn search_query_is_trimmed_and_limit_is_bounded() {
    let request = SearchRequest::new("  tampar juicy luicy  ", 500).unwrap();

    assert_eq!(request.query, "tampar juicy luicy");
    assert_eq!(request.limit, 50);
}

#[test]
fn blank_search_query_is_rejected() {
    assert!(matches!(
        SearchRequest::new("  ", 20),
        Err(CatalogError::InvalidQuery)
    ));
}

#[test]
fn automatic_quality_is_the_default() {
    assert_eq!(AudioQuality::default(), AudioQuality::Automatic);
}
