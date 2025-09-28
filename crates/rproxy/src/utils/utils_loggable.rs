// Loggable ------------------------------------------------------------

pub(crate) struct Loggable<'a>(pub &'a serde_json::Value);

impl valuable::Valuable for Loggable<'_> {
    fn as_value(&self) -> valuable::Value<'_> {
        match &self.0 {
            serde_json::Value::Null => valuable::Value::Unit,
            serde_json::Value::Bool(val) => valuable::Value::Bool(*val),
            serde_json::Value::String(val) => valuable::Value::String(val.as_str()),
            serde_json::Value::Array(_) => valuable::Value::Listable(self),
            serde_json::Value::Object(_) => valuable::Value::Mappable(self),

            serde_json::Value::Number(val) => {
                // serde_json::Value::Number is always 1 of the these 3
                if val.is_u64() {
                    valuable::Value::U64(val.as_u64().unwrap())
                } else if val.is_i64() {
                    valuable::Value::I64(val.as_i64().unwrap())
                } else {
                    valuable::Value::F64(
                        val.as_f64().expect("unexpected wrapped numeric type under the hood"),
                    )
                }
            }
        }
    }

    fn visit(&self, visitor: &mut dyn valuable::Visit) {
        match &self.0 {
            serde_json::Value::Null => visitor.visit_value(self.as_value()),
            serde_json::Value::Bool(_) => visitor.visit_value(self.as_value()),
            serde_json::Value::String(_) => visitor.visit_value(self.as_value()),
            serde_json::Value::Number(_) => visitor.visit_value(self.as_value()),

            serde_json::Value::Array(list) => {
                for val in list {
                    visitor.visit_value(Self(val).as_value());
                }
            }

            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    visitor.visit_entry(key.as_value(), Self(val).as_value());
                }
            }
        }
    }
}

impl valuable::Listable for Loggable<'_> {
    fn size_hint(&self) -> (usize, Option<usize>) {
        if let serde_json::Value::Array(arr) = &self.0 {
            return (arr.len(), Some(arr.len()));
        }
        return (0, Some(0));
    }
}

impl valuable::Mappable for Loggable<'_> {
    fn size_hint(&self) -> (usize, Option<usize>) {
        if let serde_json::Value::Object(obj) = &self.0 {
            return (obj.len(), Some(obj.len()));
        }
        return (0, Some(0));
    }
}
