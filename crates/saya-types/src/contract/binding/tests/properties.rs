use super::*;

// Property Test Generators
fn arb_col_name() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_]{0,15}"
}

fn arb_data_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("timestamp".to_string()),
        Just("timestamptz".to_string()),
        Just("date".to_string()),
        Just("time".to_string()),
        Just("datetime".to_string()),
        Just("int".to_string()),
        Just("bigint".to_string()),
        Just("numeric(10,2)".to_string()),
        Just("float".to_string()),
        Just("text".to_string()),
        Just("varchar(50)".to_string()),
        Just("boolean".to_string()),
        Just("json".to_string()),
    ]
}

fn arb_column() -> impl Strategy<Value = Column> {
    (arb_col_name(), arb_data_type(), any::<bool>()).prop_map(|(name, data_type, nullable)| {
        Column {
            name,
            data_type,
            nullable,
        }
    })
}

fn arb_table() -> impl Strategy<Value = Table> {
    (arb_col_name(), prop::collection::vec(arb_column(), 0..10)).prop_map(|(name, columns)| Table {
        name,
        columns,
        primary_key: vec![],
        foreign_keys: vec![],
    })
}

fn arb_column_requirement() -> impl Strategy<Value = ColumnRequirement> {
    prop_oneof![
        Just(ColumnRequirement::Exists),
        Just(ColumnRequirement::Time),
        Just(ColumnRequirement::Numeric),
    ]
}

fn arb_schema_binding() -> impl Strategy<Value = SchemaBinding> {
    prop_oneof![
        Just(SchemaBinding::Table),
        (arb_col_name(), arb_column_requirement()).prop_map(|(column, requirement)| {
            SchemaBinding::Column {
                column,
                requirement,
            }
        }),
        prop::collection::vec(arb_col_name(), 1..=4)
            .prop_map(|columns| SchemaBinding::Columns { columns }),
    ]
}

proptest! {
    /// Core Invariant: Appending an unrelated column with a distinct name
    /// NEVER changes the validation verdict across arbitrary generated tables.
    #[test]
    fn prop_unrelated_column_addition_never_alters_validity(
        table in arb_table(),
        binding in arb_schema_binding(),
        new_col_name in arb_col_name(),
        new_col_type in arb_data_type(),
        new_col_nullable in any::<bool>(),
    ) {
        // Ensure the new column name is distinct from what the binding depends
        // upon, for both the single-column and the multi-column shapes. Adding
        // a column that matches a missing bound column would repair the
        // binding (Invalid -> Valid), which is the opposite of "unrelated".
        match &binding {
            SchemaBinding::Column { column, .. } => {
                prop_assume!(!new_col_name.eq_ignore_ascii_case(column));
            }
            SchemaBinding::Columns { columns } => {
                for c in columns {
                    prop_assume!(!new_col_name.eq_ignore_ascii_case(c));
                }
            }
            SchemaBinding::Table => {}
        }

        let initial_verdict = binding.validate(&table);

        let mut modified_table = table.clone();
        modified_table.columns.push(Column {
            name: new_col_name,
            data_type: new_col_type,
            nullable: new_col_nullable,
        });

        let after_verdict = binding.validate(&modified_table);
        prop_assert_eq!(initial_verdict, after_verdict);
    }

    /// Dropping the referenced column from any table ALWAYS marks a Column binding Invalid.
    #[test]
    fn prop_dropped_referenced_column_always_invalid(
        table in arb_table(),
        col_name in arb_col_name(),
        requirement in arb_column_requirement(),
    ) {
        let binding = SchemaBinding::Column {
            column: col_name.clone(),
            requirement,
        };

        // Remove all columns that match the target column (case-insensitively)
        let mut stripped_table = table.clone();
        stripped_table.columns.retain(|c| !c.name.eq_ignore_ascii_case(&col_name));

        prop_assert_eq!(binding.validate(&stripped_table), BindingValidity::Invalid);
    }

    /// Arbitrary SchemaBinding round-trips identically through JSON serde.
    #[test]
    fn prop_binding_serde_round_trip(
        binding in arb_schema_binding(),
    ) {
        let serialized = serde_json::to_string(&binding).expect("serialization succeeds");
        let deserialized: SchemaBinding = serde_json::from_str(&serialized).expect("deserialization succeeds");
        prop_assert_eq!(binding, deserialized);
    }
}
