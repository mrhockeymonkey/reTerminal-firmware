//! Fills a screen-spec template with user-set values.
//!
//! The stored layout (`--state-file`) is an ordinary v1 screen spec whose
//! string values may contain placeholders; the device only ever sees the
//! result of [`render`], so the wire contract is unchanged.
//!
//! | Placeholder | Value |
//! |---|---|
//! | `{{todo}}` | [`Values::todo`] |
//! | `{{meals.0}}` … `{{meals.2}}` | breakfast, lunch, dinner |
//! | `{{to_eat}}` | one `- item` line per [`Values::to_eat`] entry |
//!
//! Substitution is textual, but every value is JSON-escaped first, so a
//! value containing quotes or newlines cannot break the document.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Most items the "To Eat" list can hold.
pub const MAX_TO_EAT: usize = 8;

/// The values substituted into the template (persisted as `values.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    /// Today's one-line todo.
    #[serde(default)]
    pub todo: String,
    /// Breakfast, lunch, dinner.
    #[serde(default)]
    pub meals: [String; 3],
    /// Up to [`MAX_TO_EAT`] items.
    #[serde(default)]
    pub to_eat: Vec<String>,
}

impl Values {
    /// Parses and checks a values document.
    pub fn from_json(json: &[u8]) -> Result<Self, String> {
        let values: Values =
            serde_json::from_slice(json).map_err(|e| format!("invalid values JSON: {e}"))?;
        if values.to_eat.len() > MAX_TO_EAT {
            return Err(format!(
                "to_eat has {} items; the limit is {MAX_TO_EAT}",
                values.to_eat.len()
            ));
        }
        Ok(values)
    }

    /// Pretty JSON, as persisted and served.
    pub fn to_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(self).expect("values serialize");
        json.push('\n');
        json
    }

    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "todo" => Some(self.todo.clone()),
            "meals.0" => Some(self.meals[0].clone()),
            "meals.1" => Some(self.meals[1].clone()),
            "meals.2" => Some(self.meals[2].clone()),
            "to_eat" => Some(
                self.to_eat
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        }
    }
}

/// Why a template could not be filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// `{{name}}` with a name [`Values`] does not provide.
    UnknownPlaceholder(String),
    /// `{{` with no matching `}}`.
    Unterminated,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::UnknownPlaceholder(name) => write!(
                f,
                "unknown placeholder {{{{{name}}}}} (known: todo, meals.0, meals.1, meals.2, to_eat)"
            ),
            TemplateError::Unterminated => write!(f, "unterminated {{{{ placeholder in template"),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Replaces every `{{name}}` in `template` with the JSON-escaped value.
pub fn render(template: &str, values: &Values) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or(TemplateError::Unterminated)?;
        let name = after[..end].trim();
        let value = values
            .lookup(name)
            .ok_or_else(|| TemplateError::UnknownPlaceholder(name.to_owned()))?;
        out.push_str(&json_escape(&value));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `value` as the inside of a JSON string literal.
fn json_escape(value: &str) -> String {
    let quoted = serde_json::to_string(value).expect("string serializes");
    quoted[1..quoted.len() - 1].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values {
            todo: "Make Stock".into(),
            meals: [
                "Cereal".into(),
                "Sandwich".into(),
                "Roast \"Chicken\"".into(),
            ],
            to_eat: vec!["ham".into(), "milk".into()],
        }
    }

    #[test]
    fn substitutes_and_escapes() {
        let t = r#"{"a":"Today: {{todo}}","b":"{{ meals.2 }}","c":"{{to_eat}}"}"#;
        let out = render(t, &values()).unwrap();
        assert_eq!(
            out,
            r#"{"a":"Today: Make Stock","b":"Roast \"Chicken\"","c":"- ham\n- milk"}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["c"], "- ham\n- milk");
    }

    #[test]
    fn empty_list_and_passthrough() {
        let v = Values::default();
        assert_eq!(render(r#""{{to_eat}}""#, &v).unwrap(), r#""""#);
        assert_eq!(render("no placeholders", &v).unwrap(), "no placeholders");
    }

    #[test]
    fn bad_placeholders() {
        let v = values();
        assert_eq!(
            render("{{nope}}", &v),
            Err(TemplateError::UnknownPlaceholder("nope".into()))
        );
        assert_eq!(render("x {{todo", &v), Err(TemplateError::Unterminated));
    }

    #[test]
    fn values_json() {
        let v = Values::from_json(br#"{"todo":"x","meals":["a","b","c"],"to_eat":[]}"#).unwrap();
        assert_eq!(v.meals[2], "c");
        assert_eq!(Values::from_json(v.to_json().as_bytes()).unwrap(), v);
        assert!(Values::from_json(br#"{"meals":["a","b"]}"#).is_err());
        let nine = format!(r#"{{"to_eat":{:?}}}"#, ["x"; 9]);
        assert!(Values::from_json(nine.as_bytes())
            .unwrap_err()
            .contains("limit is 8"));
    }
}
