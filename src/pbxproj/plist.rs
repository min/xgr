use super::graph::write_tabs;
use super::xml_escape;
use crate::spec::{Plist, ProductType, Target};
use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn info_plist_properties(
    target: &Target,
    plist: &Plist,
) -> indexmap::IndexMap<String, Value> {
    let mut properties = indexmap::IndexMap::new();
    properties.insert(
        "CFBundleIdentifier".to_owned(),
        Value::String("$(PRODUCT_BUNDLE_IDENTIFIER)".to_owned()),
    );
    properties.insert(
        "CFBundleInfoDictionaryVersion".to_owned(),
        Value::String("6.0".to_owned()),
    );
    properties.insert(
        "CFBundleName".to_owned(),
        Value::String("$(PRODUCT_NAME)".to_owned()),
    );
    properties.insert(
        "CFBundleDevelopmentRegion".to_owned(),
        Value::String("$(DEVELOPMENT_LANGUAGE)".to_owned()),
    );
    properties.insert(
        "CFBundleShortVersionString".to_owned(),
        Value::String("1.0".to_owned()),
    );
    properties.insert("CFBundleVersion".to_owned(), Value::String("1".to_owned()));
    if target.target_type != ProductType::Bundle {
        properties.insert(
            "CFBundleExecutable".to_owned(),
            Value::String("$(EXECUTABLE_NAME)".to_owned()),
        );
    }
    if let Some(package_type) = bundle_package_type(&target.target_type) {
        properties.insert(
            "CFBundlePackageType".to_owned(),
            Value::String(package_type.to_owned()),
        );
    }
    for (key, value) in &plist.attributes {
        properties.insert(key.clone(), value.clone());
    }
    properties
}

fn bundle_package_type(product_type: &ProductType) -> Option<&'static str> {
    match product_type {
        ProductType::UnitTestBundle | ProductType::UiTestBundle | ProductType::Bundle => {
            Some("BNDL")
        }
        ProductType::Application | ProductType::Watch2App => Some("APPL"),
        ProductType::Framework => Some("FMWK"),
        ProductType::XpcService | ProductType::AppExtension => Some("XPC!"),
        _ => None,
    }
}

pub(super) fn plist_xml(properties: &indexmap::IndexMap<String, Value>) -> String {
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ");
    output.push_str("\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    output.push_str("<plist version=\"1.0\">\n");
    write_plist_dict(&mut output, properties, 0);
    output.push_str("</plist>\n");
    output
}

fn write_plist_dict(
    output: &mut String,
    values: &indexmap::IndexMap<String, Value>,
    indent: usize,
) {
    write_tabs(output, indent);
    output.push_str("<dict>\n");
    for (key, value) in values {
        write_tabs(output, indent + 1);
        let _ = writeln!(output, "<key>{}</key>", xml_escape(key));
        write_plist_value(output, value, indent + 1);
    }
    write_tabs(output, indent);
    output.push_str("</dict>\n");
}

fn write_plist_value(output: &mut String, value: &Value, indent: usize) {
    write_tabs(output, indent);
    match value {
        Value::Bool(true) => output.push_str("<true/>\n"),
        Value::Bool(false) => output.push_str("<false/>\n"),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            let _ = writeln!(output, "<integer>{number}</integer>");
        }
        Value::Number(number) => {
            let _ = writeln!(output, "<real>{number}</real>");
        }
        Value::Array(items) => {
            output.push_str("<array>\n");
            for item in items {
                write_plist_value(output, item, indent + 1);
            }
            write_tabs(output, indent);
            output.push_str("</array>\n");
        }
        Value::Object(map) => {
            output.push_str("<dict>\n");
            for (key, value) in map {
                write_tabs(output, indent + 1);
                let _ = writeln!(output, "<key>{}</key>", xml_escape(key));
                write_plist_value(output, value, indent + 1);
            }
            write_tabs(output, indent);
            output.push_str("</dict>\n");
        }
        Value::Null => output.push_str("<string></string>\n"),
        Value::String(value) => {
            let _ = writeln!(output, "<string>{}</string>", xml_escape(value));
        }
    }
}
