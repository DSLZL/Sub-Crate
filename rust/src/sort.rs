use crate::{api, body::Body, json as codec, state::Store, tokens};
use hyper::Method;
use serde_json::{Value, json};

pub fn dispatch(method: &Method, path: &str, body: &Body, store: &mut Store) -> Option<api::Reply> {
    let key = path.strip_prefix("/api/sort/")?;
    if !["subs", "collections", "artifacts", "files", "tokens"].contains(&key) || method != Method::POST {
        return None;
    }
    let orders = body.clone().into_json();
    let mut items = store.list(key)?.to_vec();
    let mut failure = None;
    let mut index = |item: &Value| -> i64 {
        if item.is_null() {
            failure = Some("Cannot read properties of null (reading 'name')".to_owned());
            return -1;
        }
        let field = if key == "tokens" {
            let mut parts = Vec::new();
            for field in ["type", "name", "token"] {
                if api::cannot_coerce(&item[field]) {
                    failure = Some("Cannot convert object to primitive value".to_owned());
                    return -1;
                }
                parts.push(item.get(field).map(codec::js_string).unwrap_or_else(|| "undefined".to_owned()));
            }
            Some(json!(parts.join("-")))
        } else { item.get("name").cloned() };
        if let Some(orders) = orders.as_array() {
            return field.and_then(|field| orders.iter().position(|order| tokens::same(order, &field))).map_or(-1, |position| position as i64);
        }
        if let Some(orders) = orders.as_str() {
            let text = field.as_ref().map(codec::js_string).unwrap_or_else(|| "undefined".to_owned());
            let needle = codec::units(&text);
            if needle.is_empty() { return 0; }
            return codec::units(orders).windows(needle.len()).position(|part| part == needle).map_or(-1, |position| position as i64);
        }
        failure = Some("e.indexOf is not a function".to_owned());
        -1
    };
    items.sort_by(|left, right| index(left).cmp(&index(right)));
    if let Some(error) = failure {
        return Some(api::uncaught_error(500, &format!("TypeError: {error}")));
    }
    let mut next = store.data.clone();
    next[key] = json!(items);
    Some(api::persisted(store.commit(next), api::success(Some(json!(items)), 200)))
}
