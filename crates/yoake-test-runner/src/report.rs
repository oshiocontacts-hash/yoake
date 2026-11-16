//! The report: one JSON object, `{"ok": bool, "tests": [{"name", "ok", "ms", "error"?}], ...}`.
use serde_json::{Value, json};

/// The longest error text a record keeps, in bytes (a test or a worker cannot make the report huge).
pub const MAX_ERROR: usize = 16 * 1024;

/// `text` cut to [`MAX_ERROR`] bytes at a character boundary, with a mark where it was cut.
pub fn clip(text: &str) -> String {
    if text.len() <= MAX_ERROR {
        return text.to_owned();
    }
    let mut end = MAX_ERROR;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [cut: {} more bytes]", &text[..end], text.len() - end)
}

/// How one test ended.
#[derive(Clone, Debug, PartialEq)]
pub struct TestRecord {
    pub name: String,
    pub ok: bool,
    /// Wall-clock milliseconds the test took (as the worker measured it; the worker's whole life when it
    /// never reported).
    pub ms: u64,
    pub error: Option<String>,
}

impl TestRecord {
    pub fn pass(name: &str, ms: u64) -> Self {
        Self { name: name.to_owned(), ok: true, ms, error: None }
    }

    pub fn fail(name: &str, ms: u64, error: impl Into<String>) -> Self {
        Self { name: name.to_owned(), ok: false, ms, error: Some(clip(&error.into())) }
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({"name": self.name, "ok": self.ok, "ms": self.ms});
        if let Some(error) = &self.error {
            value["error"] = json!(error);
        }
        value
    }

    /// A worker's result file: `{"name", "ok", "ms", "error"?}`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let name = value.get("name").and_then(Value::as_str).ok_or("the result has no name")?;
        let ok = value.get("ok").and_then(Value::as_bool).ok_or("the result has no ok")?;
        let ms = value.get("ms").and_then(Value::as_u64).unwrap_or(0);
        let error = value.get("error").and_then(Value::as_str).map(clip);
        Ok(Self { name: name.to_owned(), ok, ms, error })
    }
}

/// The whole report. `extra` is merged in (the project's id and version, the notes, ...) but never
/// overrides `ok`, `tests` or `errors`. `ok` is true when no test failed and `errors` (set-up problems)
/// is empty.
pub fn report(records: &[TestRecord], errors: &[String], extra: Value) -> Value {
    let mut value = extra;
    if !value.is_object() {
        value = json!({});
    }
    let ok = errors.is_empty() && records.iter().all(|record| record.ok);
    value["ok"] = json!(ok);
    value["tests"] = Value::Array(records.iter().map(TestRecord::to_json).collect());
    value["errors"] = json!(errors);
    value
}

/// The exit code of a run: 0 all passed, 1 a test failed, 2 the run could not be set up.
pub fn exit_code(records: &[TestRecord], errors: &[String]) -> i32 {
    if !errors.is_empty() {
        2
    } else if records.iter().all(|record| record.ok) {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_has_the_documented_shape() {
        let records = [TestRecord::pass("a", 12), TestRecord::fail("b", 3, "expected stone")];
        let value = report(&records, &[], json!({"id": "demo"}));
        assert_eq!(
            value,
            json!({"id": "demo", "ok": false, "errors": [], "tests": [
                {"name": "a", "ok": true, "ms": 12},
                {"name": "b", "ok": false, "ms": 3, "error": "expected stone"},
            ]})
        );
        assert_eq!(exit_code(&records, &[]), 1);
        assert_eq!(exit_code(&records[..1], &[]), 0);
        assert!(report(&records[..1], &[], json!({}))["ok"].as_bool().unwrap());
    }

    #[test]
    fn a_setup_error_fails_the_run_even_with_passing_tests() {
        let errors = ["no game".to_owned()];
        let value = report(&[TestRecord::pass("a", 1)], &errors, json!(null));
        assert_eq!(value["ok"], json!(false));
        assert_eq!(value["errors"], json!(["no game"]));
        assert_eq!(exit_code(&[TestRecord::pass("a", 1)], &errors), 2);
    }

    #[test]
    fn extra_fields_cannot_override_the_verdict() {
        let value = report(&[TestRecord::fail("a", 1, "no")], &[], json!({"ok": true, "tests": []}));
        assert_eq!(value["ok"], json!(false));
        assert_eq!(value["tests"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn long_errors_are_cut_at_a_character_boundary() {
        assert_eq!(clip("short"), "short");
        let long = "界".repeat(MAX_ERROR);
        let clipped = clip(&long);
        assert!(clipped.len() < MAX_ERROR + 64 && clipped.contains("[cut:"), "{}", clipped.len());
        assert!(clipped.starts_with("界界"));
        let record = TestRecord::fail("t", 1, "x".repeat(MAX_ERROR * 4));
        assert!(record.error.unwrap().len() < MAX_ERROR + 64);
        let from_file = TestRecord::from_json(&json!({"name": "t", "ok": false, "error": "y".repeat(MAX_ERROR * 4)})).unwrap();
        assert!(from_file.error.unwrap().len() < MAX_ERROR + 64);
    }

    #[test]
    fn a_worker_result_round_trips() {
        let record = TestRecord::fail("a b", 7, "boom");
        assert_eq!(TestRecord::from_json(&record.to_json()), Ok(record));
        assert!(TestRecord::from_json(&json!({"name": "x"})).is_err());
        assert_eq!(TestRecord::from_json(&json!({"name": "x", "ok": true})).unwrap().ms, 0);
    }
}
