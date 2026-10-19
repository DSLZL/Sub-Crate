//! Existing token expiration rules over the shared JavaScript value operations.
use crate::{
    js_language::Js,
    script, script_network as network,
    script_pipeline::{object, optional},
};
use rquickjs::{Object, Value};

fn invalid(js: &Js<'_>, code: &str, message: String) -> rquickjs::Result<rquickjs::Error> {
    let error = Object::new(js.ctx.clone())?;
    error.set("code", code)?;
    error.set("type", "RequestInvalidError")?;
    error.set("message", message)?;
    Ok(js.ctx.throw(error.into_value()))
}

fn present<'js>(js: &Js<'js>, value: &Value<'js>) -> rquickjs::Result<bool> {
    Ok(!value.is_null() && !value.is_undefined() && !js.equal(value, &js.value("")?)?)
}

fn safe(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
}

fn milliseconds<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<f64> {
    if raw.is_number() {
        return Ok(f64::NAN);
    }
    if !raw.is_string() {
        return Err(js.error(&format!(
            "val is not a non-empty string or a valid number. val={}",
            script::serialized(&js.ctx, raw)?
        )));
    }
    if js.call::<f64, _>("number", (js.get(&raw, "length")?,))? > 100.0 {
        return Ok(f64::NAN);
    }
    let pattern = js.call("regex", (r"^(-?(?:\d+)?\.?\d+) *(milliseconds?|msecs?|ms|seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|w|years?|yrs?|y)?$", "i"))?;
    let matched = js.method(raw, "match", vec![pattern])?;
    if matched.is_null() {
        return Ok(f64::NAN);
    }
    let number: f64 = js.call("number", (js.get(&matched, "1")?,))?;
    let unit = network::prefer(js, [js.get(&matched, "2")?, js.value("ms")?])?;
    let unit = js.text(js.method(unit, "toLowerCase", vec![])?)?;
    let scale = match unit.as_str() {
        "y" | "yr" | "yrs" | "year" | "years" => 31_557_600_000.0,
        "w" | "week" | "weeks" => 604_800_000.0,
        "d" | "day" | "days" => 86_400_000.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
        "s" | "sec" | "secs" | "second" | "seconds" => 1000.0,
        _ => 1.0,
    };
    Ok(number * scale)
}

fn expiration<'js>(js: &Js<'js>, options: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let value = optional(js, &options, "mode")?;
    let mode = if !present(js, &value)? {
        if present(js, &optional(js, &options, "expiresIn")?)? {
            "duration".to_owned()
        } else if present(js, &optional(js, &options, "exp")?)? {
            "datetime".to_owned()
        } else {
            String::new()
        }
    } else if value.is_string()
        && matches!(
            js.text(value.clone())?.as_str(),
            "duration" | "datetime" | "count"
        )
    {
        js.text(value)?
    } else {
        return Err(invalid(
            js,
            "INVALID_EXPIRATION_MODE",
            format!("Unsupported expiration mode: {}", js.text(value)?),
        )?);
    };
    let output = object(js)?;
    if mode == "datetime" {
        let raw = optional(js, &options, "exp")?;
        let trimmed = if raw.is_string() {
            js.method(raw.clone(), "trim", vec![])?
        } else {
            raw.clone()
        };
        let exp: f64 = js.call("number", (trimmed.clone(),))?;
        if (!raw.is_number() && !raw.is_string())
            || js.equal(&trimmed, &js.value("")?)?
            || !safe(exp)
            || exp < 1_000_000_000_000.0
        {
            return Err(invalid(
                js,
                "INVALID_EXPIRATION_DATETIME",
                format!("Invalid exp option: {}", js.text(raw)?),
            )?);
        }
        js.set(&output, "mode", js.value(mode)?)?;
        js.set(&output, "exp", js.value(exp)?)?;
    } else if mode == "count" {
        let raw = optional(js, &options, "count")?;
        let count: f64 = js.call("number", (raw.clone(),))?;
        if !present(js, &raw)? || !safe(count) || count <= 0.0 {
            return Err(invalid(
                js,
                "INVALID_SHARE_COUNT",
                format!("Invalid count option: {}", js.text(raw)?),
            )?);
        }
        let raw = optional(js, &options, "usedCount")?;
        let raw = if raw.is_null() || raw.is_undefined() {
            js.value(0)?
        } else {
            raw
        };
        let used: f64 = js.call("number", (raw.clone(),))?;
        if !safe(used) || used < 0.0 || used > count {
            return Err(invalid(
                js,
                "INVALID_SHARE_USED_COUNT",
                format!("Invalid usedCount option: {}", js.text(raw)?),
            )?);
        }
        js.set(&output, "mode", js.value(mode)?)?;
        js.set(&output, "count", js.value(count)?)?;
        js.set(&output, "usedCount", js.value(used)?)?;
    } else {
        let raw = optional(js, &options, "expiresIn")?;
        if !present(js, &raw)? {
            if mode == "duration" {
                return Err(invalid(
                    js,
                    "INVALID_EXPIRES_IN",
                    format!("Invalid expiresIn option: {}", js.text(raw)?),
                )?);
            }
            return Ok(output);
        }
        let duration = milliseconds(js, raw.clone())?;
        if duration.is_nan() || duration <= 0.0 {
            return Err(invalid(
                js,
                "INVALID_EXPIRES_IN",
                format!("Invalid expiresIn option: {}", js.text(raw)?),
            )?);
        }
        js.set(&output, "mode", js.value("duration")?)?;
        js.set(&output, "expiresIn", raw.clone())?;
        let raw_value = optional(js, &options, "expiresValue")?;
        let raw_unit = optional(js, &options, "expiresUnit")?;
        if present(js, &raw_value)? || present(js, &raw_unit)? {
            let number: f64 = js.call("number", (raw_value.clone(),))?;
            if !number.is_finite() || number <= 0.0 {
                return Err(invalid(
                    js,
                    "INVALID_EXPIRES_VALUE",
                    format!("Invalid expiresValue option: {}", js.text(raw_value)?),
                )?);
            }
            let unit = if raw_unit.is_string() {
                js.method(raw_unit.clone(), "trim", vec![])?
            } else {
                js.value("")?
            };
            let scale = match js.text(unit.clone())?.as_str() {
                "day" => 86_400_000.0,
                "month" => 2_592_000_000.0,
                "season" => 7_776_000_000.0,
                "year" => 31_536_000_000.0,
                _ => {
                    return Err(invalid(
                        js,
                        "INVALID_EXPIRES_UNIT",
                        format!("Invalid expiresUnit option: {}", js.text(raw_unit)?),
                    )?);
                }
            };
            let fixed = js.method(js.value(number)?, "toFixed", vec![js.value(2)?])?;
            let normalized: f64 = js.call("number", (fixed,))?;
            if (duration - normalized * scale).abs() > 1.0 {
                return Err(invalid(
                    js,
                    "INVALID_DURATION_INPUT",
                    format!(
                        "expiresValue/expiresUnit do not match expiresIn option: {}{} != {}",
                        js.text(raw_value)?,
                        js.text(unit)?,
                        js.text(raw)?
                    ),
                )?);
            }
            js.set(
                &output,
                "expiresValue",
                js.value(js.text(js.value(normalized)?)?)?,
            )?;
            js.set(&output, "expiresUnit", js.method(raw_unit, "trim", vec![])?)?;
        }
        let date: Value = js.ctx.globals().get("Date")?;
        let now: f64 = js.call("number", (js.method(date, "now", vec![])?,))?;
        js.set(&output, "exp", js.value(now + duration)?)?;
    }
    Ok(output)
}

pub(crate) fn options<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    match expiration(js, input) {
        Ok(value) => Ok(value),
        Err(error) if error.is_exception() => {
            let error = js.ctx.catch();
            if js.equal(
                &optional(js, &error, "type")?,
                &js.value("RequestInvalidError")?,
            )? {
                let output = object(js)?;
                js.set(&output, "error", error)?;
                Ok(output)
            } else {
                Err(js.ctx.throw(error))
            }
        }
        Err(error) => Err(error),
    }
}
