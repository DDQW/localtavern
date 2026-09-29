//! JSON blobs stored inside rows (`AlternatesJson`, `AttachmentsJson`, `ExpressionsJson`,
//! `WorldStateJson`, ...). The C# app wrote these with default System.Text.Json, so object keys are
//! PascalCase (`{"ImagePath": ...}`), while our models use camelCase for the frontend. These
//! helpers translate key casing at the storage boundary so both apps read each other's data.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::error::Result;

pub fn to_blob<T: Serialize>(value: &T) -> Result<String> {
    let v = serde_json::to_value(value)?;
    Ok(serde_json::to_string(&recase(v, Case::Pascal))?)
}

pub fn from_blob<T: DeserializeOwned>(text: &str) -> Result<T> {
    let v: Value = serde_json::from_str(text)?;
    Ok(serde_json::from_value(recase(v, Case::Camel))?)
}

#[derive(Clone, Copy)]
enum Case {
    Pascal,
    Camel,
}

fn recase(value: Value, case: Case) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (recase_key(&k, case), recase(v, case)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| recase(v, case)).collect()),
        other => other,
    }
}

fn recase_key(key: &str, case: Case) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => {
            let head: String = match case {
                Case::Pascal => first.to_uppercase().collect(),
                Case::Camel => first.to_lowercase().collect(),
            };
            head + chars.as_str()
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        image_path: String,
        mime_type: Option<String>,
    }

    #[test]
    fn writes_pascal_case_keys_like_the_csharp_app() {
        let s = Sample { image_path: "a.png".into(), mime_type: None };
        assert_eq!(to_blob(&vec![s]).unwrap(), r#"[{"ImagePath":"a.png","MimeType":null}]"#);
    }

    #[test]
    fn reads_pascal_case_keys() {
        let parsed: Vec<Sample> = from_blob(r#"[{"ImagePath":"a.png","MimeType":"image/png"}]"#).unwrap();
        assert_eq!(parsed[0].mime_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn reads_its_own_output() {
        let s = Sample { image_path: "x".into(), mime_type: Some("m".into()) };
        let back: Sample = from_blob(&to_blob(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
