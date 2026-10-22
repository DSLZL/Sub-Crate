use crate::{Result, json, require};
use reqwest::{Method, Url, blocking::Client};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

pub struct Http(Client);

impl Http {
    pub fn new(timeout: Duration) -> Result<Self> {
        Ok(Self(
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .http1_only()
                .timeout(timeout)
                .build()?,
        ))
    }

    pub fn request(&self, base: &str, step: &Value) -> Result<Value> {
        let mut url = Url::parse(base)?;
        require(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Invalid backend URL",
        )?;
        if let Some(origin) = step
            .get("origin")
            .filter(|origin| **origin != Value::Bool(false))
        {
            if let Some(origin) = origin.as_str() {
                url = Url::parse(origin)?;
            }
            url.set_path("");
            url.set_query(None);
            url.set_fragment(None);
        }
        let path = step["path"].as_str().ok_or("Missing request path")?;
        let address = format!("{}{}", url.as_str().trim_end_matches('/'), path);
        let method = Method::from_bytes(
            step.get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .as_bytes(),
        )?;
        let mut request = self
            .0
            .request(method, address)
            .header("Content-Type", "application/json");
        if let Some(headers) = step.get("headers").and_then(Value::as_object) {
            for (key, value) in headers {
                request = request.header(key, value.as_str().ok_or("Invalid request header")?);
            }
        }
        if let Some(body) = step.get("body") {
            request = request.body(json::stringify(body));
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        let headers: BTreeMap<_, _> = response
            .headers()
            .iter()
            .map(|(key, value)| Ok((key.as_str().to_owned(), value.to_str()?.to_owned())))
            .collect::<Result<_>>()?;
        let body = String::from_utf8(response.bytes()?.to_vec())?;
        Ok(json!({"status":status,"headers":headers,"body":body}))
    }
}

pub fn check(response: &Value, expected: &Value) -> Result {
    require(
        response["status"] == expected["status"],
        "HTTP status mismatch",
    )?;
    if let Some(headers) = expected.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            require(
                response["headers"]
                    .get(key.to_lowercase())
                    .is_some_and(|actual| actual == value),
                format!("Header {key} mismatch"),
            )?;
        }
    }
    if let Some(absent) = expected.get("absent_headers").and_then(Value::as_array) {
        for key in absent {
            require(
                response["headers"]
                    .get(key.as_str().ok_or("Invalid header name")?.to_lowercase())
                    .is_none(),
                "Header must be absent",
            )?;
        }
    }
    if let Some(prefixes) = expected.get("header_prefixes").and_then(Value::as_object) {
        for (key, prefix) in prefixes {
            require(
                response["headers"]
                    .get(key.to_lowercase())
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .starts_with(prefix.as_str().ok_or("Invalid header prefix")?),
                "Header prefix mismatch",
            )?;
        }
    }
    for (key, subset) in [("json", false), ("json_contains", true)] {
        if let Some(expected) = expected.get(key) {
            json::check(
                &json::parse(response["body"].as_str().ok_or("Missing response body")?)?,
                expected,
                subset,
            )?;
        }
    }
    if let Some(body) = expected.get("body") {
        require(
            response.get("body") == Some(body),
            "Raw response body mismatch",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn response_checks_keep_headers_json_and_raw_bytes() {
        crate::actions_only();
        let response =
            json!({"status":200,"headers":{"x-test":"prefix value"},"body":"{\"flag\": false}"});
        check(&response, &json!({"status":200,"headers":{"X-Test":"prefix value"},"absent_headers":["none"],"header_prefixes":{"X-TEST":"prefix"},"json":{"flag":false},"body":"{\"flag\": false}"})).unwrap();
        for expected in [
            json!({"status":201}),
            json!({"status":200,"absent_headers":["X-Test"]}),
            json!({"status":200,"json":{"flag":0}}),
            json!({"status":200,"body":"{\"flag\":false}"}),
        ] {
            assert!(check(&response, &expected).is_err());
        }
    }

    #[test]
    fn redirects_are_observed_without_retry_or_following() {
        crate::actions_only();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut raw = vec![0; 8192];
            let count = stream.read(&mut raw).unwrap();
            assert!(String::from_utf8_lossy(&raw[..count]).starts_with("GET /redirect HTTP/1.1"));
            stream.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let result = Http::new(Duration::from_secs(5))
            .unwrap()
            .request(&origin, &json!({"path":"/redirect"}))
            .unwrap();
        assert_eq!(result["status"], 307);
        worker.join().unwrap();
    }

    #[test]
    fn early_rejection_receives_complete_json_body_without_retry() {
        use std::io::{BufRead,BufReader};
        crate::actions_only();
        let listener=TcpListener::bind("127.0.0.1:0").unwrap(); let origin=format!("http://{}",listener.local_addr().unwrap());
        let worker=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap(); stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut input=BufReader::new(stream.try_clone().unwrap()); let mut line=String::new(); input.read_line(&mut line).unwrap(); assert!(line.starts_with("POST /api/settings HTTP/1.1"));
            let mut length=None;
            loop { line.clear(); input.read_line(&mut line).unwrap(); if line=="\r\n" { break; } if let Some((key,value))=line.split_once(':') && key.eq_ignore_ascii_case("Content-Length") { length=Some(value.trim().parse::<usize>().unwrap()); } }
            stream.write_all(b"HTTP/1.0 403 Forbidden\r\nConnection: close\r\nContent-Length: 23\r\n\r\nCORS origin not allowed").unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            let mut body=vec![0;length.unwrap()]; input.read_exact(&mut body).unwrap(); assert_eq!(body,br#"{"mustNotCommit": true}"#);
            listener.set_nonblocking(true).unwrap(); assert!(listener.accept().is_err());
        });
        let response=Http::new(Duration::from_secs(5)).unwrap().request(&origin,&json!({"path":"/api/settings","method":"POST","headers":{"Origin":"https://denied.invalid"},"body":{"mustNotCommit":true}}));
        worker.join().unwrap(); assert_eq!(response.unwrap()["status"],403);
    }
}
