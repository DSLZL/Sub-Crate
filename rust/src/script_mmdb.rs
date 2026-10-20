//! Public MMDB methods over the shared native MaxMind reader.
use crate::{
    js_language::Js,
    script_pipeline::{object, optional},
};
use rquickjs::{
    Ctx, Function, Object, Value,
    prelude::{Func, Opt, This},
};

fn open<'js>(js: &Js<'js>, path: Value<'js>, kind: &str) -> rquickjs::Result<Value<'js>> {
    let read = crate::mmdb::open(js.ctx.clone(), js.text(path)?, kind.to_owned())?;
    let reader: Object = read.get("reader")?;
    let create: Function = js
        .ctx
        .eval("reader => new SubStoreMmdbUtilities.ReaderModel(reader)")?;
    create.call((reader,))
}

fn initialize_reader<'js>(
    ctx: Ctx<'js>,
    this: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let options = options
        .0
        .filter(|value| !value.is_undefined())
        .unwrap_or(object(&js)?);
    for (key, kind, env) in [
        ("country", "Country", "SUB_STORE_MMDB_COUNTRY_PATH"),
        ("asn", "ASN", "SUB_STORE_MMDB_ASN_PATH"),
    ] {
        let value = js.get(&options, key)?;
        let value = if js.truthy(&value)? {
            value
        } else {
            std::env::var(env)
                .ok()
                .map(|value| js.value(value))
                .transpose()?
                .unwrap_or_else(|| js.undefined())
        };
        if js.truthy(&value)? {
            js.set(
                &this,
                if key == "country" {
                    "countryReader"
                } else {
                    "asnReader"
                },
                open(&js, value, kind)?,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn initialize<'js>(ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(include_str!("../vendor/mmdb-utilities.js"))?;
    ctx.globals()
        .set("__initMMDB", Func::from(initialize_reader))?;
    let constructor: Function =
        ctx.eval("(function MMDB(options) { __initMMDB(this, options); })")?;
    let prototype: Object = constructor.get("prototype")?;
    for (name, reader, method, field) in [
        ("geoip", "countryReader", "country", "isoCode"),
        ("ipaso", "asnReader", "asn", "autonomousSystemOrganization"),
        ("ipasn", "asnReader", "asn", "autonomousSystemNumber"),
    ] {
        prototype.set(
            name,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, This(this): This<Value<'js>>, ip: Opt<Value<'js>>| {
                    let js = Js::new(ctx);
                    let reader = js.get(&this, reader)?;
                    if reader.is_null() || reader.is_undefined() {
                        return Ok(js.undefined());
                    }
                    let record =
                        js.method(reader, method, vec![ip.0.unwrap_or_else(|| js.undefined())])?;
                    if method == "country" {
                        optional(&js, &optional(&js, &record, "country")?, field)
                    } else {
                        optional(&js, &record, field)
                    }
                },
            )?,
        )?;
    }
    utils.set("MMDB", constructor)
}
