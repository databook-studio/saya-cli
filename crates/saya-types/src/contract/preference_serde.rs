//! Hand-rolled `Deserialize` for `PreferenceValue`.
//!
//! The derived `Deserialize` would populate the string-carrying variants'
//! fields directly and bypass the validated constructors — exactly the
//! "validated constructor beside a publicly-constructible variant" the security
//! standard warns about. This impl routes deserialization through the
//! constructors, so a `{"kind":"timezone","value":"SELECT..."}` row is refused
//! by the *type*, not only by the store's admission gate. That is the spec's
//! "make it unrepresentable, not filtered" rule, enforced on every construction
//! path, not just the obvious one.

use serde::Deserialize;

use crate::contract::preference::{DateGrain, OutputStyle, PreferenceValue};

impl<'de> Deserialize<'de> for PreferenceValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Kind,
            Value,
            Grain,
            Style,
            Name,
        }

        struct PreferenceValueVisitor;

        impl<'de> serde::de::Visitor<'de> for PreferenceValueVisitor {
            type Value = PreferenceValue;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a preference value tagged with `kind`")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut kind: Option<String> = None;
                let mut value: Option<String> = None;
                let mut grain: Option<DateGrain> = None;
                let mut style: Option<OutputStyle> = None;
                let mut name: Option<String> = None;
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Kind => {
                            if kind.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        Field::Value => {
                            if value.is_some() {
                                return Err(serde::de::Error::duplicate_field("value"));
                            }
                            value = Some(map.next_value()?);
                        }
                        Field::Grain => {
                            if grain.is_some() {
                                return Err(serde::de::Error::duplicate_field("grain"));
                            }
                            grain = Some(map.next_value()?);
                        }
                        Field::Style => {
                            if style.is_some() {
                                return Err(serde::de::Error::duplicate_field("style"));
                            }
                            style = Some(map.next_value()?);
                        }
                        Field::Name => {
                            if name.is_some() {
                                return Err(serde::de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                    }
                }
                let kind = kind.ok_or_else(|| serde::de::Error::missing_field("kind"))?;
                // Construction goes through the validated constructors, so a
                // malformed timezone or profile name is refused here, at the
                // type, before any caller ever holds a `PreferenceValue`.
                match kind.as_str() {
                    "timezone" => {
                        let v = value.ok_or_else(|| serde::de::Error::missing_field("value"))?;
                        PreferenceValue::timezone(v).map_err(serde::de::Error::custom)
                    }
                    "date_grain" => {
                        let g = grain.ok_or_else(|| serde::de::Error::missing_field("grain"))?;
                        Ok(PreferenceValue::date_grain(g))
                    }
                    "output_style" => {
                        let s = style.ok_or_else(|| serde::de::Error::missing_field("style"))?;
                        Ok(PreferenceValue::output_style(s))
                    }
                    "default_profile" => {
                        let n = name.ok_or_else(|| serde::de::Error::missing_field("name"))?;
                        PreferenceValue::default_profile(n).map_err(serde::de::Error::custom)
                    }
                    other => Err(serde::de::Error::unknown_variant(
                        other,
                        &["timezone", "date_grain", "output_style", "default_profile"],
                    )),
                }
            }
        }

        deserializer.deserialize_map(PreferenceValueVisitor)
    }
}
