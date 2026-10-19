//! ECMAScript primitives for native processors; business dispatch stays in Rust.
use rquickjs::{
    Array, Ctx, Exception, FromJs, Function, IntoJs, Object, Value, function::IntoArgs,
};

#[derive(Clone)]
pub struct Js<'js> {
    pub ctx: Ctx<'js>,
}

impl<'js> Js<'js> {
    pub fn new(ctx: Ctx<'js>) -> Self {
        Self { ctx }
    }

    pub fn call<R: FromJs<'js>, A: IntoArgs<'js>>(
        &self,
        name: &str,
        args: A,
    ) -> rquickjs::Result<R> {
        let primitives: Object = self.ctx.globals().get("__language")?;
        let function: Function = primitives.get(name)?;
        function.call(args)
    }

    pub fn value<T: IntoJs<'js>>(&self, value: T) -> rquickjs::Result<Value<'js>> {
        value.into_js(&self.ctx)
    }

    pub fn undefined(&self) -> Value<'js> {
        Value::new_undefined(self.ctx.clone())
    }

    pub fn get(&self, value: &Value<'js>, key: &str) -> rquickjs::Result<Value<'js>> {
        self.call("get", (value.clone(), key))
    }

    pub fn set(&self, value: &Value<'js>, key: &str, item: Value<'js>) -> rquickjs::Result<()> {
        self.call("set", (value.clone(), key, item))
    }

    pub fn text(&self, value: Value<'js>) -> rquickjs::Result<String> {
        self.call("text", (value,))
    }

    pub fn truthy(&self, value: &Value<'js>) -> rquickjs::Result<bool> {
        self.call("truthy", (value.clone(),))
    }

    pub fn equal(&self, a: &Value<'js>, b: &Value<'js>) -> rquickjs::Result<bool> {
        self.call("equal", (a.clone(), b.clone()))
    }

    pub fn array(
        &self,
        values: impl IntoIterator<Item = Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let array = Array::new(self.ctx.clone())?;
        for (index, value) in values.into_iter().enumerate() {
            array.set(index, value)?;
        }
        Ok(array.into_value())
    }

    pub fn method(
        &self,
        value: Value<'js>,
        name: &str,
        args: Vec<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        self.call("method", (value, name, self.array(args)?))
    }

    pub fn map<F>(&self, value: Value<'js>, callback: F) -> rquickjs::Result<Value<'js>>
    where
        F: Fn(Value<'js>) -> rquickjs::Result<Value<'js>> + 'js,
    {
        let function = Function::new(self.ctx.clone(), callback)?;
        self.method(value, "map", vec![function.into_value()])
    }

    pub fn filter<F>(&self, value: Value<'js>, callback: F) -> rquickjs::Result<Value<'js>>
    where
        F: Fn(Value<'js>, usize) -> rquickjs::Result<bool> + 'js,
    {
        let function = Function::new(self.ctx.clone(), callback)?;
        self.method(value, "filter", vec![function.into_value()])
    }

    pub fn each<F>(&self, value: Value<'js>, callback: F) -> rquickjs::Result<()>
    where
        F: Fn(Value<'js>) -> rquickjs::Result<()> + 'js,
    {
        let function = Function::new(self.ctx.clone(), callback)?;
        self.method(value, "forEach", vec![function.into_value()])?;
        Ok(())
    }

    pub fn sort<F>(&self, value: Value<'js>, callback: F) -> rquickjs::Result<Value<'js>>
    where
        F: Fn(Value<'js>, Value<'js>) -> rquickjs::Result<i32> + 'js,
    {
        let function = Function::new(self.ctx.clone(), callback)?;
        self.method(value, "sort", vec![function.into_value()])
    }

    pub fn json_clone(&self, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
        let text = self.ctx.json_stringify(value)?;
        self.call(
            "jsonParse",
            (text.map_or_else(|| self.undefined(), |text| text.into_value()),),
        )
    }

    pub fn regex(&self, mut pattern: Value<'js>, flags: &str) -> rquickjs::Result<Value<'js>> {
        let insensitive: bool = self.call("starts", (pattern.clone(), "(?i)"))?;
        if insensitive {
            pattern = self.method(pattern, "substring", vec![self.value(4)?])?;
        }
        self.call(
            "regex",
            (
                pattern,
                format!("{}{flags}", if insensitive { "i" } else { "" }),
            ),
        )
    }

    pub fn regex_test(&self, expression: Value<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
        let result = self.method(expression, "test", vec![value])?;
        self.truthy(&result)
    }

    pub fn error(&self, message: &str) -> rquickjs::Error {
        Exception::throw_message(&self.ctx, message)
    }
}

// The two ordered tables are data from utils/geo.js, including its KOP-XIAO and
// surgioproject attribution. No old application function is evaluated here.
pub fn get_flag<'js>(js: &Js<'js>, name: Value<'js>) -> rquickjs::Result<String> {
    let expression: Value = js.call(
        "regex",
        (r"[\uD83C][\uDDE6-\uDDFF][\uD83C][\uDDE6-\uDDFF]", ""),
    )?;
    let found = js.method(name.clone(), "match", vec![expression])?;
    let fallback = if js.truthy(&found)? {
        js.text(js.get(&found, "0")?)?
    } else {
        "🏴‍☠️".to_owned()
    };
    let data: Object = js.ctx.globals().get("__geo")?;
    for table in ["names", "iso"] {
        let rows: Array = data.get(table)?;
        for row in rows.iter::<Array>() {
            let row = row?;
            let flag: String = row.get(0)?;
            let words: Array = row.get(1)?;
            for word in words.iter::<String>() {
                let word = word?;
                let pattern = if table == "iso" {
                    format!("(^|[^a-zA-Z]){word}([^a-zA-Z]|$)")
                } else {
                    word
                };
                let regex: Value =
                    js.call("regex", (pattern, if table == "iso" { "" } else { "i" }))?;
                if !js.regex_test(regex, name.clone())? {
                    continue;
                }
                if table == "iso" && flag == "🇨🇳" {
                    let regex: Value = js.call("regex", ("(^|[^a-zA-Z])CN2([^a-zA-Z]|$)", ""))?;
                    if js.regex_test(regex, name.clone())? {
                        break;
                    }
                }
                if table == "names" && flag == "🇲🇳" {
                    let regex: Value = js.call("regex", ("内蒙古", ""))?;
                    if js.regex_test(regex, name.clone())? {
                        return Ok("🇨🇳".to_owned());
                    }
                }
                return Ok(flag);
            }
        }
    }
    Ok(fallback)
}

pub fn remove_flag<'js>(js: &Js<'js>, name: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let expression: Value = js.call(
        "regex",
        (r"[\uD83C][\uDDE6-\uDDFF][\uD83C][\uDDE6-\uDDFF]|🏴‍☠️|🏳️‍🌈", "g"),
    )?;
    let result = js.method(name, "replace", vec![expression, js.value("")?])?;
    js.method(result, "trim", vec![])
}
