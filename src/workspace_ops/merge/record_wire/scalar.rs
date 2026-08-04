use serde_yaml::value::{Tag as ValueTag, TaggedValue};
use serde_yaml::{Number, Value};
use yaml_rust2::parser::Tag;
use yaml_rust2::scanner::TScalarStyle;

pub(super) struct ScalarNode {
    pub(super) value: Value,
    pub(super) merge_key: bool,
}

pub(super) fn parse_scalar(
    value: String,
    style: TScalarStyle,
    tag: Option<Tag>,
) -> Result<ScalarNode, String> {
    let core = tag.as_ref().and_then(core_tag);
    let merge_key =
        core == Some("merge") || (tag.is_none() && style == TScalarStyle::Plain && value == "<<");
    let inferred = if core != Some("str") && (core.is_some() || style == TScalarStyle::Plain) {
        infer_plain_scalar(&value, core)?
    } else {
        Value::String(value)
    };
    Ok(ScalarNode {
        value: tagged_value(inferred, tag.filter(|tag| core_tag(tag).is_none())),
        merge_key,
    })
}

fn infer_plain_scalar(value: &str, core: Option<&str>) -> Result<Value, String> {
    match core {
        Some("bool") => parse_bool(value)
            .map(Value::Bool)
            .ok_or_else(|| "invalid explicitly tagged boolean".to_owned()),
        Some("int") => parse_integer(value)
            .map(Value::Number)
            .ok_or_else(|| "invalid explicitly tagged integer".to_owned()),
        Some("float") => parse_float(value)
            .map(|value| Value::Number(Number::from(value)))
            .ok_or_else(|| "invalid explicitly tagged float".to_owned()),
        Some("null") => parse_null(value)
            .then_some(Value::Null)
            .ok_or_else(|| "invalid explicitly tagged null".to_owned()),
        Some(_) => Ok(Value::String(value.to_owned())),
        None => {
            if value.is_empty() || parse_null(value) {
                Ok(Value::Null)
            } else if let Some(value) = parse_bool(value) {
                Ok(Value::Bool(value))
            } else if let Some(value) = parse_integer(value) {
                Ok(Value::Number(value))
            } else if !digits_but_not_number(value) {
                Ok(parse_float(value).map_or_else(
                    || Value::String(value.to_owned()),
                    |value| Value::Number(Number::from(value)),
                ))
            } else {
                Ok(Value::String(value.to_owned()))
            }
        }
    }
}

pub(super) fn tagged_value(value: Value, tag: Option<Tag>) -> Value {
    let Some(tag) = tag else { return value };
    Value::Tagged(Box::new(TaggedValue {
        tag: ValueTag::new(format!("{}{}", tag.handle, tag.suffix)),
        value,
    }))
}

fn core_tag(tag: &Tag) -> Option<&str> {
    (tag.handle == "tag:yaml.org,2002:").then_some(tag.suffix.as_str())
}

fn parse_null(value: &str) -> bool {
    matches!(value, "null" | "Null" | "NULL" | "~")
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

fn parse_integer(value: &str) -> Option<Number> {
    let unsigned = value.strip_prefix('+').unwrap_or(value);
    if !value.starts_with('-') {
        if let Some((digits, base)) = radix_digits(unsigned) {
            return u64::from_str_radix(digits, base).ok().map(Number::from);
        }
        if !digits_but_not_number(value) {
            return unsigned.parse::<u64>().ok().map(Number::from);
        }
    }
    for (prefix, base) in [("-0x", 16), ("-0o", 8), ("-0b", 2)] {
        if let Some(digits) = value.strip_prefix(prefix) {
            return i64::from_str_radix(&format!("-{digits}"), base)
                .ok()
                .map(Number::from);
        }
    }
    (!digits_but_not_number(value))
        .then(|| value.parse::<i64>().ok().map(Number::from))
        .flatten()
}

fn radix_digits(value: &str) -> Option<(&str, u32)> {
    value
        .strip_prefix("0x")
        .map(|rest| (rest, 16))
        .or_else(|| value.strip_prefix("0o").map(|rest| (rest, 8)))
        .or_else(|| value.strip_prefix("0b").map(|rest| (rest, 2)))
}

fn digits_but_not_number(value: &str) -> bool {
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    value.len() > 1
        && value.starts_with('0')
        && value[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_float(value: &str) -> Option<f64> {
    let unsigned = value.strip_prefix('+').unwrap_or(value);
    match unsigned {
        ".inf" | ".Inf" | ".INF" => Some(f64::INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        _ if matches!(value, "-.inf" | "-.Inf" | "-.INF") => Some(f64::NEG_INFINITY),
        _ => unsigned
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite()),
    }
}
