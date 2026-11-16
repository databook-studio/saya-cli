use super::*;

#[test]
fn spec_name_and_description_bounds() {
    assert!(is_valid_param_name("region"));
    assert!(is_valid_param_name("_x"));
    assert!(is_valid_param_name("p0"));
    assert!(is_valid_param_name(&"a".repeat(32)));
    for bad in [
        "",
        "Region",
        "9lives",
        "has space",
        "has-dash",
        "dot.name",
        "é",
        &"a".repeat(33),
        "\u{1}",
    ] {
        assert!(!is_valid_param_name(bad), "{bad:?} must be refused");
    }
    ParameterSpec::new("a", ParamType::String, true, None).expect("a bare spec validates");
    ParameterSpec::new(
        "a",
        ParamType::String,
        true,
        Some("d".repeat(MAX_PARAM_DESCRIPTION_BYTES)),
    )
    .expect("a description at the cap fits");
    let over = ParameterSpec::new(
        "a",
        ParamType::String,
        true,
        Some("d".repeat(MAX_PARAM_DESCRIPTION_BYTES + 1)),
    );
    assert_eq!(over, Err(ParamError::DescriptionTooLong));
    let control = ParameterSpec::new("a", ParamType::String, true, Some("bad\u{1}".to_owned()));
    assert_eq!(control, Err(ParamError::DescriptionControlCharacter));
    let bad_name = ParameterSpec::new("Region", ParamType::String, true, None);
    assert_eq!(bad_name, Err(ParamError::InvalidName("Region".to_owned())));
    // A struct built without `new` is still checked by `validate`.
    let raw = spec_named("Region");
    assert_eq!(
        raw.validate(),
        Err(ParamError::InvalidName("Region".to_owned()))
    );
}

#[test]
fn spec_list_bounds_and_uniqueness() {
    let specs = |count: usize| {
        (0..count)
            .map(|i| spec_named(&format!("p{i:02}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(ParameterSpec::validate_list(&[]), Ok(()));
    ParameterSpec::validate_list(&specs(MAX_PARAMETERS)).expect("32 distinct specs fit");
    assert_eq!(
        ParameterSpec::validate_list(&specs(MAX_PARAMETERS + 1)),
        Err(ParamError::TooManySpecs(MAX_PARAMETERS + 1))
    );
    let duplicates = vec![spec_named("region"), spec_named("region")];
    assert_eq!(
        ParameterSpec::validate_list(&duplicates),
        Err(ParamError::DuplicateName("region".to_owned()))
    );
    let misnamed = vec![spec_named("region"), spec_named("Region")];
    assert!(matches!(
        ParameterSpec::validate_list(&misnamed),
        Err(ParamError::InvalidName(_))
    ));
}
