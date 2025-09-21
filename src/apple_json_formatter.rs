use serde_json::Value;
use std::io::{self, Write};

/// Formats JSON with Apple's preferred style for .xcstrings files:
/// - Spaces before colons
/// - 2-space indentation
/// - Preserves key order when using IndexMap
pub fn to_apple_format(value: &Value) -> String {
    let mut buffer = Vec::new();
    write_value(&mut buffer, value, 0).expect("Failed to write JSON");
    String::from_utf8(buffer).expect("Invalid UTF-8")
}

fn write_value<W: Write>(writer: &mut W, value: &Value, indent_level: usize) -> io::Result<()> {
    match value {
        Value::Null => write!(writer, "null"),
        Value::Bool(b) => write!(writer, "{}", b),
        Value::Number(n) => write!(writer, "{}", n),
        Value::String(s) => write!(writer, "\"{}\"", escape_string(s)),
        Value::Array(arr) => write_array(writer, arr, indent_level),
        Value::Object(obj) => write_object(writer, obj, indent_level),
    }
}

fn write_array<W: Write>(writer: &mut W, array: &[Value], indent_level: usize) -> io::Result<()> {
    if array.is_empty() {
        return write!(writer, "[]");
    }

    writeln!(writer, "[")?;
    for (i, value) in array.iter().enumerate() {
        write_indent(writer, indent_level + 1)?;
        write_value(writer, value, indent_level + 1)?;
        if i < array.len() - 1 {
            write!(writer, ",")?;
        }
        writeln!(writer)?;
    }
    write_indent(writer, indent_level)?;
    write!(writer, "]")
}

fn write_object<W: Write>(
    writer: &mut W,
    obj: &serde_json::Map<String, Value>,
    indent_level: usize,
) -> io::Result<()> {
    if obj.is_empty() {
        return write!(writer, "{{}}");
    }

    writeln!(writer, "{{")?;
    let entries: Vec<_> = obj.iter().collect();
    for (i, (key, value)) in entries.iter().enumerate() {
        write_indent(writer, indent_level + 1)?;
        // Apple format: space before colon
        write!(writer, "\"{}\" : ", escape_string(key))?;
        write_value(writer, value, indent_level + 1)?;
        if i < entries.len() - 1 {
            write!(writer, ",")?;
        }
        writeln!(writer)?;
    }
    write_indent(writer, indent_level)?;
    write!(writer, "}}")
}

fn write_indent<W: Write>(writer: &mut W, level: usize) -> io::Result<()> {
    for _ in 0..level {
        write!(writer, "  ")?; // 2 spaces per indent level
    }
    Ok(())
}

fn escape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\u{0008}' => result.push_str("\\b"),
            '\u{000C}' => result.push_str("\\f"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_apple_format() {
        let value = json!({
            "version": "1.0",
            "sourceLanguage": "en",
            "strings": {
                "hello": {
                    "localizations": {
                        "en": {
                            "stringUnit": {
                                "state": "translated",
                                "value": "Hello"
                            }
                        }
                    }
                }
            }
        });

        let formatted = to_apple_format(&value);
        assert!(formatted.contains("\"version\" : \"1.0\""));
        assert!(formatted.contains("\"sourceLanguage\" : \"en\""));
        assert!(formatted.contains("\"state\" : \"translated\""));
    }

    #[test]
    fn test_empty_objects_and_arrays() {
        let value = json!({
            "empty_object": {},
            "empty_array": [],
            "nested": {
                "also_empty": {}
            }
        });

        let formatted = to_apple_format(&value);
        assert!(formatted.contains("\"empty_object\" : {}"));
        assert!(formatted.contains("\"empty_array\" : []"));
        assert!(formatted.contains("\"also_empty\" : {}"));
    }

    #[test]
    fn test_string_escaping() {
        let value = json!({
            "test": "Line 1\nLine 2\t\"quoted\""
        });

        let formatted = to_apple_format(&value);
        assert!(formatted.contains("Line 1\\nLine 2\\t\\\"quoted\\\""));
    }
}
