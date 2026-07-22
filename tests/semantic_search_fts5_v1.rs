//! FTS5 search behavior tests. Covers flatten_json (Task 3), the migration
//! (Task 5), and end-to-end search (Task 10). Tests are appended task-by-task.

use llm_wiki::semantic::flatten_json;

#[test]
fn flatten_json_string_is_itself() {
    let v = serde_json::json!("Reinvent the Wheel");
    assert_eq!(flatten_json(&v), "Reinvent the Wheel");
}

#[test]
fn flatten_json_number_stringifies() {
    let v = serde_json::json!(42);
    assert_eq!(flatten_json(&v), "42");
}

#[test]
fn flatten_json_bool_stringifies() {
    assert_eq!(flatten_json(&serde_json::json!(true)), "true");
    assert_eq!(flatten_json(&serde_json::json!(false)), "false");
}

#[test]
fn flatten_json_array_space_joins_elements() {
    let v = serde_json::json!(["Tesla", "BMW", 7]);
    assert_eq!(flatten_json(&v), "Tesla BMW 7");
}

#[test]
fn flatten_json_object_space_joins_values() {
    let v = serde_json::json!({"a": "x", "b": "y"});
    // Object iteration order is insertion order for serde_json, but assert
    // set-style to stay robust to any future value ordering choice.
    let flat = flatten_json(&v);
    assert!(flat.contains("x") && flat.contains("y"), "got: {flat}");
}

#[test]
fn flatten_json_null_is_empty() {
    assert_eq!(flatten_json(&serde_json::Value::Null), "");
}

#[test]
fn flatten_json_nested_array_recurses() {
    let v = serde_json::json!([["a", "b"], "c"]);
    assert_eq!(flatten_json(&v), "a b c");
}
