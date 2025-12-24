use serde_json::Value;

pub fn json_deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(t), Value::Object(s)) => {
            for (k, v) in s {
                json_deep_merge(t.entry(k).or_insert(Value::Null), v);
            }
        }
        (Value::Array(t), Value::Array(s)) => {
            t.extend(s);
        }
        (t, s) => *t = s,
    }
}
