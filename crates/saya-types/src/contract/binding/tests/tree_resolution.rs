use super::*;

/// Tree Resolution Test: Tests case-insensitive catalog/schema/table resolution in SchemaTree.
#[test]
fn test_validate_in_tree_resolution() {
    let table = make_table("Rental", &[("Return_Date", "timestamp")]);
    let tree = SchemaTree {
        databases: vec![Database {
            name: "MainDb".to_string(),
            schemas: vec![Schema {
                name: "Public".to_string(),
                tables: vec![table],
            }],
        }],
    };

    let binding = SchemaBinding::Column {
        column: "return_date".to_string(),
        requirement: ColumnRequirement::Time,
    };

    // Exact match
    assert_eq!(
        binding.validate_in_tree(&tree, "MainDb", "Public", "Rental"),
        BindingValidity::Valid
    );

    // Case-insensitive match on all 3 components
    assert_eq!(
        binding.validate_in_tree(&tree, "maindb", "public", "rental"),
        BindingValidity::Valid
    );

    // Missing catalog
    assert_eq!(
        binding.validate_in_tree(&tree, "other_db", "public", "rental"),
        BindingValidity::Invalid
    );

    // Missing schema
    assert_eq!(
        binding.validate_in_tree(&tree, "maindb", "other_schema", "rental"),
        BindingValidity::Invalid
    );

    // Missing table
    assert_eq!(
        binding.validate_in_tree(&tree, "maindb", "public", "customer"),
        BindingValidity::Invalid
    );
}
