#[test]
fn documentation_matches_registered_schemas() {
    assert_eq!(
        polaris_tools::tool_catalog(),
        include_str!("../../../docs/tool-catalog.md")
    );
}
