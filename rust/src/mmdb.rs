//! Contained native MMDB reads; the source class retains its public projections.
use maxminddb::Reader;
use rquickjs::{Coerced, Ctx, Exception, FromJs, Function, Object, Value};
use std::{io, net::IpAddr, path::Path, sync::Arc};

fn reject<'js, T>(ctx: &Ctx<'js>, name: &str, message: &str) -> rquickjs::Result<T> {
    let error = Exception::from_message(ctx.clone(), message)?;
    error.as_object().set("name", name)?;
    Err(error.throw())
}

fn load<'js>(ctx: &Ctx<'js>, path: String, kind: &str) -> rquickjs::Result<Reader<Vec<u8>>> {
    if kind == "ASN" && !Path::new(&path).exists() {
        return reject(ctx, "Error", "GeoLite2 ASN MMDB does not exist");
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let error = Exception::from_message(
                ctx.clone(),
                &format!("ENOENT: no such file or directory, open '{path}'"),
            )?;
            error.as_object().set("code", "ENOENT")?;
            error.as_object().set("path", path)?;
            return Err(error.throw());
        }
        Err(error) => return reject(ctx, "Error", &error.to_string()),
    };
    match Reader::from_source(bytes) {
        Ok(reader) => Ok(reader),
        Err(error) => reject(ctx, "InvalidDbBufferError", &error.to_string()),
    }
}

struct Record {
    address: String,
    data: Option<serde_json::Value>,
    network: Option<String>,
}

fn lookup<'js>(
    ctx: &Ctx<'js>,
    reader: &Reader<Vec<u8>>,
    value: Value<'js>,
    kind: Option<&str>,
) -> rquickjs::Result<Record> {
    let valid_type = value.is_string();
    let array_type = value.is_array();
    let raw = Coerced::<String>::from_js(ctx, value)?.0;
    let ip = match raw.parse::<IpAddr>() {
        Ok(ip) => ip,
        _ => return reject(ctx, "ValueError", &format!("{raw} is invalid")),
    };
    if let Some(kind) = kind {
        let method = if kind == "ASN" { "asn()" } else { "country()" };
        let database_type = &reader.metadata().database_type;
        if !database_type.contains(kind) {
            return reject(
                ctx,
                "BadMethodCallError",
                &format!("The {method} method cannot be used with the {database_type} database"),
            );
        }
    }
    if array_type {
        return Err(Exception::throw_type(ctx, "input.split is not a function"));
    }
    let result = reader
        .lookup(ip)
        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))?;
    let record = result
        .decode::<serde_json::Value>()
        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))?;
    match record {
        Some(_) if !valid_type => reject(
            ctx,
            "Error",
            "Invalid argument: Addr or parsable string expected",
        ),
        data => {
            let network = if data.is_some() && kind.is_none() {
                Some(
                    result
                        .network()
                        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))?
                        .to_string(),
                )
            } else {
                None
            };
            Ok(Record {
                address: raw,
                data,
                network,
            })
        }
    }
}

pub fn open<'js>(ctx: Ctx<'js>, path: String, kind: String) -> rquickjs::Result<Function<'js>> {
    if kind != "Country" && kind != "ASN" {
        return Err(Exception::throw_type(&ctx, "Invalid MMDB reader kind"));
    }
    let reader = Arc::new(load(&ctx, path, &kind)?);
    let model = native_reader(ctx.clone(), Arc::clone(&reader))?;
    let read = Function::new(ctx, move |ctx: Ctx<'js>, value: Value<'js>| {
        let record = lookup(&ctx, &reader, value, Some(&kind))?;
        match record.data {
            Some(data) => Ok(data.to_string()),
            None => reject(
                &ctx,
                "AddressNotFoundError",
                &format!("The address {} is not in the database", record.address),
            ),
        }
    })?;
    read.set("reader", model)?;
    Ok(read)
}

fn validate<'js>(ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
    let raw = Coerced::<String>::from_js(&ctx, value)?.0;
    Ok(raw.parse::<IpAddr>().is_ok())
}

fn native_reader<'js>(
    ctx: Ctx<'js>,
    reader: Arc<Reader<Vec<u8>>>,
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    let metadata = serde_json::json!({"databaseType": reader.metadata().database_type});
    object.set("metadata", ctx.json_parse(metadata.to_string())?)?;
    object.set("validate", Function::new(ctx.clone(), validate)?)?;
    object.set(
        "getWithNetwork",
        Function::new(ctx, move |ctx: Ctx<'js>, value: Value<'js>| {
            let record = lookup(&ctx, &reader, value, None)?;
            ctx.json_parse(serde_json::json!([record.data, record.network]).to_string())
        })?,
    )?;
    Ok(object)
}
