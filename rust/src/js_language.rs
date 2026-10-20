//! ECMAScript primitives for native processors; business dispatch stays in Rust.
use rquickjs::{
    Array, Coerced, Ctx, Exception, FromJs, Function, IntoJs, Object, Value, function::IntoArgs,
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
        if value.is_null() || value.is_undefined() {
            return Err(Exception::throw_type(
                &self.ctx,
                &format!(
                    "Cannot read properties of {} (reading '{key}')",
                    if value.is_null() { "null" } else { "undefined" }
                ),
            ));
        }
        if let Some(object) = value.as_object() {
            return object.get(key);
        }
        self.call("get", (value.clone(), key))
    }

    pub fn set(&self, value: &Value<'js>, key: &str, item: Value<'js>) -> rquickjs::Result<()> {
        self.call("set", (value.clone(), key, item))
    }

    pub fn text(&self, value: Value<'js>) -> rquickjs::Result<String> {
        self.call("text", (value,))
    }

    pub fn truthy(&self, value: &Value<'js>) -> rquickjs::Result<bool> {
        Ok(Coerced::<bool>::from_js(&self.ctx, value.clone())?.0)
    }

    pub fn equal(&self, a: &Value<'js>, b: &Value<'js>) -> rquickjs::Result<bool> {
        // SAFETY: The engine borrows live handles from this context; strict equality
        // does not coerce, throw or take ownership. Rust Eq compares handle bits.
        Ok(unsafe {
            rquickjs::qjs::JS_IsStrictEqual(self.ctx.as_raw().as_ptr(), a.as_raw(), b.as_raw())
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize(ctx: &Ctx<'_>) {
        ctx.globals()
            .set("SubStoreScriptUtilities", Object::new(ctx.clone()).unwrap())
            .unwrap();
        ctx.eval::<(), _>(include_str!("script-bootstrap.js"))
            .unwrap();
    }

    #[test]
    fn native_operators_match_js_without_coercing_user_objects() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx);
            let js = Js::new(ctx.clone());
            let values: Array = ctx
                .eval(
                    r#"(() => {
                      const object = {[Symbol.toPrimitive](){throw new Error('coerced');}};
                      const symbol = Symbol('same');
                      const revoked = Proxy.revocable({}, {}); revoked.revoke();
                      return [undefined,null,false,true,0,-0,1,NaN,Infinity,-Infinity,
                        0n,1n,BigInt('1'),'','same',['sa','me'].join(''),'\0','\ud800','\udfff',
                        symbol,symbol,Symbol('same'),object,object,{},[],new Boolean(false),
                        revoked.proxy,()=>{}];
                    })()"#,
                )
                .unwrap();
            let values: Vec<_> = values.iter::<Value>().collect::<Result<_, _>>().unwrap();
            for value in &values {
                assert_eq!(
                    js.truthy(value).unwrap(),
                    js.call::<bool, _>("truthy", (value.clone(),)).unwrap()
                );
                for other in &values {
                    assert_eq!(
                        js.equal(value, other).unwrap(),
                        js.call::<bool, _>("equal", (value.clone(), other.clone()))
                            .unwrap()
                    );
                }
            }
        });
    }

    #[test]
    fn native_property_reads_keep_receivers_errors_and_primitive_boxing() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx);
            let js = Js::new(ctx.clone());
            let object: Value = ctx
                .eval(
                    r#"(() => {
                      globalThis.reads = [];
                      const prototype = {get inherited(){reads.push(this === proxy);return this.marker;}};
                      const target = Object.assign(Object.create(prototype), {marker:7,'\0':8,'\ue000':9});
                      const proxy = new Proxy(target, {get(target,key,receiver){
                        reads.push(key);return Reflect.get(target,key,receiver);
                      }});
                      return proxy;
                    })()"#,
                )
                .unwrap();
            for key in ["inherited", "missing", "\0", "\u{e000}"] {
                let value = js.get(&object, key).unwrap();
                let expected = js.call("get", (object.clone(), key)).unwrap();
                assert!(js.equal(&value, &expected).unwrap());
            }
            assert_eq!(
                ctx.eval::<String, _>("JSON.stringify(reads)").unwrap(),
                "[\"inherited\",true,\"marker\",\"inherited\",true,\"marker\",\"missing\",\"missing\",\"\\u0000\",\"\\u0000\",\"\u{e000}\",\"\u{e000}\"]"
            );
            let throwing: Value = ctx
                .eval("globalThis.original = {}; ({get value(){throw original;}})")
                .unwrap();
            assert!(js.get(&throwing, "value").is_err());
            let thrown = ctx.catch();
            assert!(
                js.equal(&thrown, &ctx.globals().get("original").unwrap())
                    .unwrap()
            );
            for source in ["'中🙂'", "true", "1", "1n", "Symbol('s')", "()=>{}"] {
                let value: Value = ctx.eval(source).unwrap();
                let actual = js.get(&value, "toString").unwrap();
                let expected = js.call("get", (value, "toString")).unwrap();
                assert!(js.equal(&actual, &expected).unwrap());
            }
            for source in ["null", "undefined"] {
                let value: Value = ctx.eval(source).unwrap();
                assert!(js.get(&value, "name").is_err());
                let error = ctx.catch();
                assert_eq!(
                    error
                        .as_object()
                        .unwrap()
                        .get::<_, String>("message")
                        .unwrap(),
                    format!("Cannot read properties of {source} (reading 'name')")
                );
                assert!(crate::script_pipeline::optional(&js, &value, "name")
                    .unwrap()
                    .is_undefined());
            }
            let frozen: Value = ctx.eval("Object.freeze({value:1})").unwrap();
            js.set(&frozen, "value", js.value(2).unwrap()).unwrap();
            js.call::<(), _>("remove", (frozen.clone(), "value")).unwrap();
            assert_eq!(js.get(&frozen, "value").unwrap().as_int(), Some(1));
        });
    }
}
