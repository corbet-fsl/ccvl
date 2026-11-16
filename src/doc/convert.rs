//! JSON <-> Yrs `Any` conversion and the canonical shape dump used by the vectors.
use std::collections::HashMap;

use serde_json::{Map, Number as JsonNumber, Value};
use yrs::any::Number;
use yrs::types::ToJson;
use yrs::{Any, Array, GetString, Map as YMapTrait, Out, ReadTxn};

/// Convert a JSON value to `Any` the way Yjs encodes a JS value: integral numbers
/// in the safe range become integers, everything else a float.
pub(crate) fn value_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => Any::Number(Number::try_i64(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Any::String(s.as_str().into()),
        Value::Array(items) => {
            Any::Array(items.iter().map(value_to_any).collect::<Vec<_>>().into())
        }
        Value::Object(map) => Any::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), value_to_any(v)))
                .collect::<HashMap<_, _>>()
                .into(),
        ),
    }
}

fn number_to_value(n: Number) -> Value {
    match n {
        Number::Int(i) => Value::from(i),
        Number::Float(f) => JsonNumber::from_f64(f).map_or(Value::Null, Value::Number),
    }
}

/// Convert `Any` to JSON; integral floats collapse to integers so that values
/// compare equal with what `JSON.parse` yields for the same document.
pub(crate) fn any_to_value(any: &Any) -> Value {
    match any {
        Any::Null | Any::Undefined | Any::Buffer(_) => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(n) => number_to_value(*n),
        Any::String(s) => Value::String(s.to_string()),
        Any::Array(items) => Value::Array(items.iter().map(any_to_value).collect()),
        Any::Map(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), any_to_value(v)))
                .collect::<Map<_, _>>(),
        ),
    }
}

/// The `toJSON` projection of any shared value (Y.Text flattens to its string).
pub(crate) fn out_to_value<T: ReadTxn>(txn: &T, out: &Out) -> Value {
    any_to_value(&out.to_json(txn))
}

/// Canonical structural dump: it keeps what `toJSON` hides, namely which values
/// are Y.Map, Y.Array or Y.Text and which are plain `Any` values.
///
/// `Y.Map` -> `{"$map": {...}}`, `Y.Array` -> `{"$array": [...]}`,
/// `Y.Text` -> `{"$text": "..."}`, a plain array or object -> `{"$any": ...}`,
/// primitives as themselves.
pub(crate) fn dump_shape<T: ReadTxn>(txn: &T, out: &Out) -> Value {
    dump_shape_at(txn, out, 0)
}

/// Nesting deeper than this is not dumped (the dump is a test and tooling aid).
const SHAPE_DEPTH: usize = 256;

fn dump_shape_at<T: ReadTxn>(txn: &T, out: &Out, depth: usize) -> Value {
    if depth > SHAPE_DEPTH {
        return Value::Null;
    }
    match out {
        Out::YMap(m) => {
            let inner: Map<String, Value> = m
                .iter(txn)
                .map(|(k, v)| (k.to_string(), dump_shape_at(txn, &v, depth + 1)))
                .collect();
            let mut o = Map::new();
            o.insert("$map".into(), Value::Object(inner));
            Value::Object(o)
        }
        Out::YArray(a) => {
            let inner: Vec<Value> = a
                .iter(txn)
                .map(|v| dump_shape_at(txn, &v, depth + 1))
                .collect();
            let mut o = Map::new();
            o.insert("$array".into(), Value::Array(inner));
            Value::Object(o)
        }
        Out::YText(t) => {
            let mut o = Map::new();
            o.insert("$text".into(), Value::String(t.get_string(txn)));
            Value::Object(o)
        }
        Out::Any(any) => {
            let v = any_to_value(any);
            if v.is_array() || v.is_object() {
                let mut o = Map::new();
                o.insert("$any".into(), v);
                Value::Object(o)
            } else {
                v
            }
        }
        _ => Value::Null,
    }
}
