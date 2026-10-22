//! Named protocol response records shared by service adapters.
use crate::json::{self, Value};

pub struct UserList {
    pub url: String,
    pub name: String,
    pub count: Option<u64>,
    pub group: &'static str,
}

impl UserList {
    pub fn into_json(self) -> Value {
        let mut fields =
            vec![("url", Value::str(self.url)), ("name", Value::str(self.name)), ("group", Value::str(self.group))];
        if let Some(count) = self.count {
            fields.push(("count", Value::Num(count as f64)));
        }
        json::obj(fields)
    }
}

pub struct Quality {
    pub id: String,
    pub name: String,
    pub label: String,
}

impl Quality {
    pub fn into_json(self) -> Value {
        json::obj(vec![("id", Value::str(self.id)), ("name", Value::str(self.name)), ("label", Value::str(self.label))])
    }
}
