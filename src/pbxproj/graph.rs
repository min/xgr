use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub(super) struct PbxObject {
    pub(super) isa: &'static str,
    pub(super) comment: Option<String>,
    pub(super) fields: BTreeMap<String, PbxValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PbxValue {
    Int(i64),
    String(String),
    Ref { id: String, comment: Option<String> },
    Array(Vec<PbxValue>),
    Dict(BTreeMap<String, PbxValue>),
}

#[derive(Debug, Default)]
pub(super) struct PbxGraph {
    pub(super) objects: BTreeMap<String, PbxObject>,
    pub(super) comments: HashMap<String, String>,
}

impl PbxGraph {
    pub(super) fn add(&mut self, key: &str, object: PbxObject) -> String {
        let id = self.id_for(key);
        if !self.objects.contains_key(&id) {
            if let Some(comment) = object.comment.clone() {
                self.comments.insert(id.clone(), comment);
            }
            self.objects.insert(id.clone(), object);
        }
        id
    }

    pub(super) fn add_or_merge_group(&mut self, key: &str, object: PbxObject) -> String {
        let id = self.id_for(key);
        let Some(existing) = self.objects.get_mut(&id) else {
            if let Some(comment) = object.comment.clone() {
                self.comments.insert(id.clone(), comment);
            }
            self.objects.insert(id.clone(), object);
            return id;
        };

        if let Some(PbxValue::Array(existing_children)) = existing.fields.get_mut("children") {
            if let Some(PbxValue::Array(new_children)) = object.fields.get("children") {
                for child in new_children {
                    if !existing_children.contains(child) {
                        existing_children.push(child.clone());
                    }
                }
            }
        }

        if !existing.fields.contains_key("name") {
            if let Some(name) = object.fields.get("name") {
                existing.fields.insert("name".to_owned(), name.clone());
            }
        }
        if !existing.fields.contains_key("path") {
            if let Some(path) = object.fields.get("path") {
                existing.fields.insert("path".to_owned(), path.clone());
            }
        }

        id
    }

    pub(super) fn id_for(&self, key: &str) -> String {
        object_id(key, 0)
    }
}

impl PbxObject {
    pub(super) fn new(isa: &'static str, comment: impl Into<String>) -> Self {
        Self {
            isa,
            comment: Some(comment.into()),
            fields: BTreeMap::new(),
        }
    }

    pub(super) fn field(mut self, key: &str, value: PbxValue) -> Self {
        self.fields.insert(key.to_owned(), value);
        self
    }
}

impl PbxValue {
    pub(super) fn reference(id: String, comment: impl Into<String>) -> Self {
        Self::Ref {
            id,
            comment: Some(comment.into()),
        }
    }

    pub(super) fn uncommented_reference(id: String) -> Self {
        Self::Ref {
            id,
            comment: Some(String::new()),
        }
    }

    pub(super) fn write(
        &self,
        output: &mut String,
        indent: usize,
        comments: &HashMap<String, String>,
        id_map: &HashMap<String, String>,
    ) {
        match self {
            Self::Int(value) => {
                let _ = write!(output, "{value}");
            }
            Self::String(value) => output.push_str(&quote(value)),
            Self::Ref { id, comment } => {
                output.push_str(&mapped_id(id, id_map));
                let comment = match comment {
                    Some(comment) if comment.is_empty() => None,
                    Some(comment) => Some(comment.clone()),
                    None => comments.get(id).cloned(),
                };
                if let Some(comment) = comment {
                    let _ = write!(output, " /* {comment} */");
                }
            }
            Self::Array(values) => {
                if values.is_empty() {
                    output.push_str("(\n");
                    write_tabs(output, indent);
                    output.push(')');
                } else {
                    output.push_str("(\n");
                    for value in values {
                        write_tabs(output, indent + 1);
                        value.write(output, indent + 1, comments, id_map);
                        output.push_str(",\n");
                    }
                    write_tabs(output, indent);
                    output.push(')');
                }
            }
            Self::Dict(values) => {
                if values.is_empty() {
                    output.push_str("{\n");
                    write_tabs(output, indent);
                    output.push('}');
                } else {
                    output.push_str("{\n");
                    let mut values = values.iter().collect::<Vec<_>>();
                    values.sort_by(|(a, _), (b, _)| mapped_id(a, id_map).cmp(&mapped_id(b, id_map)));
                    for (key, value) in values {
                        write_tabs(output, indent + 1);
                        let key = mapped_id(key, id_map);
                        output.push_str(&quote_pbx_key(&key));
                        output.push_str(" = ");
                        value.write(output, indent + 1, comments, id_map);
                        output.push_str(";\n");
                    }
                    write_tabs(output, indent);
                    output.push('}');
                }
            }
        }
    }
}

pub(super) fn object_id(key: &str, salt: u64) -> String {
    let mut hash = 0xcbf29ce484222325u64 ^ salt;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let second = hash.rotate_left(17) ^ 0x9e3779b97f4a7c15;
    format!("{hash:016X}{:08X}", second as u32)
}

pub(super) fn mapped_id<'a>(id: &'a str, id_map: &'a HashMap<String, String>) -> Cow<'a, str> {
    match id_map.get(id) {
        Some(mapped) => Cow::Borrowed(mapped.as_str()),
        None => Cow::Borrowed(id),
    }
}

pub(super) fn write_compact_value(
    value: &PbxValue,
    output: &mut String,
    comments: &HashMap<String, String>,
    id_map: &HashMap<String, String>,
    field_key: &str,
) {
    match value {
        PbxValue::Int(value) => {
            let _ = write!(output, "{value}");
        }
        PbxValue::String(value) => {
            if field_key == "remoteGlobalIDString" {
                output.push_str(&mapped_id(value, id_map));
            } else {
                output.push_str(&quote(value));
            }
        }
        PbxValue::Ref { id, comment } => {
            output.push_str(&mapped_id(id, id_map));
            let comment = match comment {
                Some(comment) if comment.is_empty() => None,
                Some(comment) => Some(comment.clone()),
                None => comments.get(id).cloned(),
            };
            if let Some(comment) = comment {
                let _ = write!(output, " /* {comment} */");
            }
        }
        PbxValue::Array(values) => {
            output.push('(');
            for value in values {
                write_compact_value(value, output, comments, id_map, field_key);
                output.push_str(", ");
            }
            output.push(')');
        }
        PbxValue::Dict(values) => {
            output.push('{');
            for (key, value) in values {
                let _ = write!(output, "{key} = ");
                write_compact_value(value, output, comments, id_map, key);
                output.push_str("; ");
            }
            output.push('}');
        }
    }
}

pub(super) fn xcode_reference_acronym(isa: &str) -> String {
    isa.replace("PBX", "")
        .replace("XC", "")
        .chars()
        .filter(|c| c.to_lowercase().to_string() != c.to_string())
        .collect()
}

pub(super) fn phase_name(object: &PbxObject) -> Option<String> {
    match object.isa {
        "PBXSourcesBuildPhase" => Some("Sources".to_owned()),
        "PBXResourcesBuildPhase" => Some("Resources".to_owned()),
        "PBXFrameworksBuildPhase" => Some("Frameworks".to_owned()),
        "PBXHeadersBuildPhase" => Some("Headers".to_owned()),
        _ => match object.fields.get("name") {
            Some(PbxValue::String(value)) => Some(value.clone()),
            _ => object.comment.clone().filter(|value| !value.is_empty()),
        },
    }
}

pub(super) fn quote_pbx_key(value: &str) -> String {
    if value.len() == 24 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        value.to_owned()
    } else {
        quote(value)
    }
}

pub(super) fn quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    let bare_ok = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '$'));
    if bare_ok {
        value.to_owned()
    } else {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        )
    }
}

pub(super) fn pbx_value_from_json(value: &Value) -> PbxValue {
    match value {
        Value::Bool(true) => PbxValue::String("YES".to_owned()),
        Value::Bool(false) => PbxValue::String("NO".to_owned()),
        Value::Number(number) => PbxValue::String(number.to_string()),
        Value::String(value) => PbxValue::String(value.clone()),
        Value::Array(values) => PbxValue::Array(values.iter().map(pbx_value_from_json).collect()),
        Value::Object(map) => PbxValue::Dict(
            map.iter()
                .map(|(key, value)| (key.clone(), pbx_value_from_json(value)))
                .collect(),
        ),
        Value::Null => PbxValue::String(String::new()),
    }
}

const PBX_TABS: &str = "\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t";

pub(super) fn write_tabs(output: &mut String, indent: usize) {
    if indent <= PBX_TABS.len() {
        output.push_str(&PBX_TABS[..indent]);
    } else {
        for _ in 0..indent {
            output.push('\t');
        }
    }
}
