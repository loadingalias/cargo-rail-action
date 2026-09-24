use std::collections::BTreeSet;

use serde::de::{self, Deserialize as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::{ActionError, Result};

#[derive(Debug)]
struct UniqueValue(Value);

impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(UniqueValue)
                    .ok_or_else(|| E::custom("JSON number is not finite"))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_string(value.to_string())
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }

            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some((key, value)) = object.next_entry::<String, UniqueValue>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate JSON object key {key:?}")));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueVisitor)
    }
}

/// Parse exactly one JSON value, rejecting duplicate object keys at every depth.
pub(crate) fn parse_unique_json(bytes: &[u8], subject: &str) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = UniqueValue::deserialize(&mut deserializer)
        .map_err(|error| ActionError::rejected(format!("{subject} is invalid JSON: {error}")))?;
    deserializer
        .end()
        .map_err(|error| ActionError::rejected(format!("{subject} has trailing data: {error}")))?;
    Ok(parsed.0)
}

pub(crate) fn exact_keys(
    object: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
    subject: &str,
) -> Result<()> {
    let required = required.iter().copied().collect::<BTreeSet<_>>();
    let optional = optional.iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing = required.difference(&actual).copied().collect::<Vec<_>>();
    let unknown = actual
        .difference(&required)
        .filter(|field| !optional.contains(**field))
        .copied()
        .collect::<Vec<_>>();
    require(missing.is_empty(), format!("{subject} is missing {missing:?}"))?;
    require(unknown.is_empty(), format!("{subject} has unknown fields {unknown:?}"))
}

pub(crate) fn object_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    subject: &str,
) -> Result<&'a Map<String, Value>> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} is missing or invalid")))
}

pub(crate) fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ActionError::rejected(message))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_unique_json;

    #[test]
    fn duplicate_json_keys_are_rejected_at_every_depth() {
        let error =
            parse_unique_json(br#"{"outer":{"value":1,"value":2}}"#, "fixture").expect_err("duplicate key must fail");
        assert!(error.to_string().contains("duplicate"));
    }
}
