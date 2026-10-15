//! Contained native MMDB reads; the source class retains its public projections.
use maxminddb::Reader;
use rquickjs::{Coerced, Ctx, Exception, FromJs, Function, Value};
use std::{io, net::IpAddr, path::Path};

fn reject<'js, T>(ctx: &Ctx<'js>, name: &str, message: &str) -> rquickjs::Result<T> {
    let error = Exception::from_message(ctx.clone(), message)?;
    error.as_object().set("name", name)?;
    Err(error.throw())
}

pub fn open<'js>(ctx: Ctx<'js>, path: String, kind: String) -> rquickjs::Result<Function<'js>> {
    if kind != "Country" && kind != "ASN" {
        return Err(Exception::throw_type(&ctx, "Invalid MMDB reader kind"));
    }
    if kind == "ASN" && !Path::new(&path).exists() {
        return reject(&ctx, "Error", "GeoLite2 ASN MMDB does not exist");
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
        Err(error) => return reject(&ctx, "Error", &error.to_string()),
    };
    let reader = match Reader::from_source(bytes) {
        Ok(reader) => reader,
        Err(error) => return reject(&ctx, "InvalidDbBufferError", &error.to_string()),
    };
    Function::new(ctx, move |ctx: Ctx<'js>, value: Value<'js>| {
        let valid_type = value.is_string();
        let array_type = value.is_array();
        let raw = Coerced::<String>::from_js(&ctx, value)?.0;
        let ip = match raw.parse::<IpAddr>() {
            Ok(ip) => ip,
            _ => return reject(&ctx, "ValueError", &format!("{raw} is invalid")),
        };
        let method = if kind == "ASN" { "asn()" } else { "country()" };
        let database_type = &reader.metadata().database_type;
        if !database_type.contains(&kind) {
            return reject(
                &ctx,
                "BadMethodCallError",
                &format!("The {method} method cannot be used with the {database_type} database"),
            );
        }
        if array_type {
            return reject(&ctx, "TypeError", "input.split is not a function");
        }
        let record = reader
            .lookup(ip)
            .and_then(|result| result.decode::<serde_json::Value>())
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        match record {
            Some(_) if !valid_type => reject(
                &ctx,
                "Error",
                "Invalid argument: Addr or parsable string expected",
            ),
            Some(record) => Ok(record.to_string()),
            None => reject(
                &ctx,
                "AddressNotFoundError",
                &format!("The address {raw} is not in the database"),
            ),
        }
    })
}
