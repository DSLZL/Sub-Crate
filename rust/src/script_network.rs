//! Shared source-loader request queue and URL/header decoding.
use crate::js_language::Js;
use rquickjs::{Array, Function, Object, Promise, Value};
use std::{collections::VecDeque, sync::Mutex, time::Duration};
use tokio::sync::oneshot;

struct Queue {
    active: usize,
    pending: VecDeque<oneshot::Sender<()>>,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    active: 0,
    pending: VecDeque::new(),
});

struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        let mut queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
        queue.active = queue.active.saturating_sub(1);
        if let Some(next) = queue.pending.pop_front() {
            let _ = next.send(());
        }
    }
}

pub fn host<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    js.ctx.globals().get("$substore")
}

pub fn settings<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    let value = js.method(host(js)?, "read", vec![js.value("settings")?])?;
    prefer(js, [value, Object::new(js.ctx.clone())?.into_value()])
}

pub fn prefer<'js>(
    js: &Js<'js>,
    values: impl IntoIterator<Item = Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let mut last = js.undefined();
    for value in values {
        last = value;
        if js.truthy(&last)? {
            break;
        }
    }
    Ok(last)
}

fn setting_integer<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    minimum: f64,
    default: f64,
) -> rquickjs::Result<f64> {
    let number: f64 = js.call("number", (value,))?;
    Ok(if number.is_finite() && number.fract() == 0.0 && number >= minimum {
        number
    } else {
        default
    })
}

async fn acquire(js: &Js<'_>) -> rquickjs::Result<Slot> {
    loop {
        let settings = settings(js)?;
        let limit = setting_integer(
            js,
            js.get(&settings, "backendRequestConcurrency")?,
            1.0,
            10.0,
        )?;
        let wait = setting_integer(
            js,
            js.get(&settings, "backendRequestConcurrencyWaitTime")?,
            0.0,
            0.0,
        )?;
        let pending = {
            let mut queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
            if (queue.active as f64) < limit {
                queue.active += 1;
                None
            } else {
                let (sender, receiver) = oneshot::channel();
                queue.pending.push_back(sender);
                Some(receiver)
            }
        };
        if let Some(pending) = pending {
            let _ = pending.await;
        } else {
            let slot = Slot;
            if wait > 0.0 {
                // The source queue uses setTimeout, including its large-delay fallback.
                let milliseconds = if wait > 2147483647.0 { 1 } else { wait as u64 };
                tokio::time::sleep(Duration::from_millis(milliseconds)).await;
            }
            return Ok(slot);
        }
    }
}

pub async fn request<'js>(
    js: &Js<'js>,
    method: &str,
    options: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let _slot = acquire(js).await?;
    let factory: Function = js.ctx.globals().get("createHostHTTP")?;
    let http = factory.call(())?;
    let result = js.method(http, method, vec![options])?;
    let promise: Promise = js.call("resolve", (result,))?;
    promise.into_future().await
}

pub fn fragment<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<(Value<'js>, Value<'js>)> {
    let parts = js.method(raw, "split", vec![js.value("#")?])?;
    let url = js.get(&parts, "0")?;
    let length: usize = js.call("length", (parts.clone(),))?;
    let mut arguments = Object::new(js.ctx.clone())?.into_value();
    if length > 1 {
        let raw = js.get(&parts, "1")?;
        let decoded = (|| {
            let decoder: Function = js.ctx.globals().get("decodeURIComponent")?;
            let text: Value = decoder.call((raw.clone(),))?;
            js.call("jsonParse", (text,))
        })();
        match decoded {
            Ok(value) => arguments = value,
            Err(_) => {
                js.ctx.catch();
                let pairs = js.method(raw, "split", vec![js.value("&")?])?;
                let pairs: Vec<Value> = js.call("iterable", (pairs,))?;
                for pair in pairs {
                    let pair = js.method(pair, "split", vec![js.value("=")?])?;
                    let key = js.get(&pair, "0")?;
                    let value = js.get(&pair, "1")?;
                    let value = if value.is_null()
                        || value.is_undefined()
                        || js.equal(&value, &js.value("")?)?
                    {
                        js.value(true)?
                    } else {
                        let decoder: Function = js.ctx.globals().get("decodeURIComponent")?;
                        decoder.call((value,))?
                    };
                    js.call::<(), _>("set", (arguments.clone(), key, value))?;
                }
            }
        }
    }
    Ok((url, arguments))
}

pub fn headers<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    user_agent: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let value = if value.is_string() {
        js.call("jsonParse", (value,))?
    } else {
        value
    };
    let kind: String = js.call("typeOf", (value.clone(),))?;
    if kind != "object" || value.is_null() || value.is_array() {
        return Ok(js.undefined());
    }
    let object = js.ctx.globals().get("Object")?;
    let keys = js.method(object, "keys", vec![value.clone()])?;
    let length: usize = js.call("length", (keys,))?;
    if length == 0 {
        return Ok(js.undefined());
    }
    let headers = Object::new(js.ctx.clone())?.into_value();
    js.set(&headers, "user-agent", user_agent)?;
    let keys: Array = js.call("keysIn", (value.clone(),))?;
    for key in keys.iter::<Value>() {
        let key = key?;
        let lower = js.method(key.clone(), "toLowerCase", vec![])?;
        let value: Value = js.call("get", (value.clone(), key))?;
        js.call::<(), _>("set", (headers.clone(), lower, value))?;
    }
    Ok(headers)
}
