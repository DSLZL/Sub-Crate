use crate::{Result, api::Api, fixture::quote, json, raw_http, require, sharing};
use base64::Engine;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write, path::Path};

pub fn etag(body: &[u8]) -> String {
    format!(
        "W/\"{:x}-{}\"",
        body.len(),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, body).as_ref()
        )
    )
}
fn compress(body: &[u8], coding: &str) -> Result<Vec<u8>> {
    if coding == "gzip" {
        let mut encoder = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::default());
        encoder.write_all(body)?;
        Ok(encoder.finish()?)
    } else {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body)?;
        Ok(encoder.finish()?)
    }
}
pub struct Requests<'a> {
    pub api: &'a Api,
    pub base: &'a str,
    pub output: &'a Path,
    pub label: &'a str,
    pub origin: Option<&'a str>,
    pub observed: Vec<Value>,
}
impl Requests<'_> {
    pub fn send(&mut self, body: &[u8], spec: Value) -> Result<Value> {
        let path = spec["path"].as_str().unwrap_or("/api/settings");
        let method = spec["method"].as_str().unwrap_or("PATCH");
        let status = spec["status"].as_u64().unwrap_or(200);
        let mut headers = BTreeMap::new();
        if let Some(origin) = self.origin {
            headers.insert("Origin".into(), origin.into());
        }
        if let Some(fields) = spec["headers"].as_object() {
            for (key, value) in fields {
                headers.insert(
                    key.to_owned(),
                    value
                        .as_str()
                        .ok_or("Invalid raw fixture header")?
                        .to_owned(),
                );
            }
        }
        let media = spec.get("media").unwrap_or(&Value::Null);
        if spec.get("media").is_none() {
            headers.insert("Content-Type".into(), "application/json".into());
        } else if let Some(media) = media.as_str() {
            headers.insert("Content-Type".into(), media.to_owned());
        }
        let before = [
            std::fs::read(self.api.state.join("sub-store.json"))?,
            std::fs::read(self.api.state.join("root.json"))?,
        ];
        let chunks = spec["chunk_size"]
            .as_u64()
            .map(usize::try_from)
            .transpose()?;
        let reply =
            raw_http::request(self.base, method, path, &headers, body, chunks)?;
        let after = [
            std::fs::read(self.api.state.join("sub-store.json"))?,
            std::fs::read(self.api.state.join("root.json"))?,
        ];
        let state = self.api.durable()?;
        let mut record = reply.evidence();
        record["request"] = json!({"spec":spec,"headers":headers,"body_base64":base64::engine::general_purpose::STANDARD.encode(body),"chunk_sizes":chunks.map(|size|body.chunks(size).map(<[u8]>::len).collect::<Vec<_>>())});
        record["state"] = state.clone();
        record["root"] = self.api.stored("root.json")?;
        record["files_before_base64"] = json!(
            before
                .iter()
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
                .collect::<Vec<_>>()
        );
        record["files_after_base64"] = json!(
            after
                .iter()
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
                .collect::<Vec<_>>()
        );
        json::write_new(
            self.output
                .join(format!("{}-{:04}.json", self.label, self.observed.len())),
            &record,
        )?;
        require(
            u64::from(reply.status) == status,
            format!(
                "{} {method} {path}: expected {status}, actual {} {:?}",
                self.label,
                reply.status,
                String::from_utf8_lossy(&reply.body)
            ),
        )?;
        if [400, 413, 415].contains(&status) || spec["parser_error"] == true {
            sharing::diagnostic(
                &reply.value()?,
                "production",
                "unused",
                reply.status,
                "",
                true,
                false,
            )?;
            require(
                !reply.headers.contains_key("etag"),
                "Parser error must not have ETag",
            )?;
        }
        if status >= 400 || method == "GET" || method == "OPTIONS" {
            require(
                before == after,
                "Parser/read/rejection changed durable state",
            )?;
        }
        let compared = [
            "content-type",
            "content-length",
            "etag",
            "content-security-policy",
            "x-content-type-options",
        ]
        .iter()
        .filter_map(|key| {
            reply
                .headers
                .get(*key)
                .map(|value| (key.to_string(), json!(value)))
        })
        .collect::<serde_json::Map<_, _>>();
        self.observed.push(json!({"method":method,"path":path,"status":status,"headers":compared,"body_base64":base64::engine::general_purpose::STANDARD.encode(&reply.body),"state":state}));
        if (status < 400 || spec["json_error"] == true) && !reply.body.is_empty() {
            json::parse(std::str::from_utf8(&reply.body)?)
        } else {
            Ok(Value::Null)
        }
    }
}
pub fn run(api: &Api, base: &str, output: &Path, label: &str, origin: Option<&str>) -> Result<Vec<Value>> {
    let original = api.durable()?;
    let mut request = Requests {
        api,
        base,
        output,
        label,
        origin,
        observed: Vec::new(),
    };
    for plural in ["subs", "collections"] {
        let path = format!("/api/{plural}");
        request.send(b"[]", json!({"path":path,"method":"PUT"}))?;
        request.send(
            &raw_http::json_bytes(&json!({"name":format!("parser-{plural}")})),
            json!({"path":path,"method":"POST","status":201}),
        )?;
        for (body,media) in [(b"{}".as_slice(),Some("application/json")),(b"{\"forEach\":0,\"age-public-key\":\"invalid\",\"0\":{\"name\":\"must-not-replace\"}}",Some("application/json")),(b"{\"type\":\"Buffer\",\"data\":[65,0,255]}",Some("application/json")),(b"name=must-not-replace&forEach[]=one&forEach[]=two",Some("application/x-www-form-urlencoded")),(b"not-json",None),(b"not-json",Some("invalid")),(b"",Some("application/json")),(b"",Some("application/x-www-form-urlencoded"))] {request.send(body,json!({"path":path,"method":"PUT","media":media,"status":500}))?;let reply=request.observed.last().ok_or("Missing rejected replacement")?;let bytes=base64::engine::general_purpose::STANDARD.decode(reply["body_base64"].as_str().ok_or("Missing rejection bytes")?)?;json::check(&json::parse(std::str::from_utf8(&bytes)?)?,&json!({"status":"failed","error":{"message":"r.forEach is not a function"}}),false)?;let expected_length=bytes.len().to_string();require(reply["headers"]["content-type"]=="text/plain; charset=utf-8"&&reply["headers"]["content-length"]==expected_length&&reply["headers"]["etag"]==etag(&bytes),"Rejected replacement header/ETag mismatch")?;}
    }
    for (body, media) in [
        (b"".as_slice(), Some("application/json")),
        (b"", Some("text/plain")),
        (b"not-json", None),
        (b"not-json", Some("invalid")),
        (b"not-json", Some("application/json; broken")),
    ] {
        request.send(body, json!({"media":media}))?;
    }
    for (charset, body) in [
        (
            "utf-8",
            b"\xef\xbb\xbf{\"parserUnicode\":\"\xe4\xb8\xad\"}".to_vec(),
        ),
        (
            "UTF-16LE",
            "{\"parserUnicode\":\"UTF16🙂\"}"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        ),
        ("utf-8", b"{\"parserLossy\":\"\xff\"}".to_vec()),
    ] {
        let data = request.send(
            &body,
            json!({"media":format!("Application/JSON; charset=\"{charset}\"")}),
        )?["data"]
            .clone();
        require(
            data["parserLossy"] == "�"
                || data["parserUnicode"] == "中"
                || data["parserUnicode"] == "UTF16🙂",
            "Body charset/UTF8 replacement mismatch",
        )?;
    }
    for charset in ["utf-16be", "utf-16", "utf-7", "utf-7imap"] {
        request.send(b"{}",json!({"media":format!("application/json; charset={charset}"),"status":403,"parser_error":true}))?;
        request.send(
            b"",
            json!({"media":format!("application/json; charset={charset}")}),
        )?;
    }
    for body in [
        b"{".as_slice(),
        b"null",
        b"false",
        b"0",
        b"\"text\"",
        b" \t\r\n",
    ] {
        request.send(body, json!({"status":400}))?;
    }
    for media in [
        "application/json; charset=latin1",
        "application/json; charset=utf-99",
        "application/json; charset=utf-32",
        "application/x-www-form-urlencoded; charset=iso-8859-1",
    ] {
        request.send(b"{}", json!({"media":media,"status":415}))?;
    }
    request.send(
        b"{}",
        json!({"headers":{"Content-Encoding":"br"},"status":415}),
    )?;
    for path in ["/api/utils/env", "/api/missing-parser-route"] {
        request.send(b"{", json!({"path":path,"method":"GET","status":400}))?;
    }
    if origin.is_none() {
        request.send(b"{", json!({"path":"/","method":"GET","status":400}))?;
    }
    for media in [
        "application/json",
        "text/plain",
        "application/x-www-form-urlencoded",
    ] {
        request.send(
            &vec![b'x'; 102401],
            json!({"method":"GET","media":media,"status":413}),
        )?;
    }
    request.send(
        &raw_http::json_bytes(&json!({"large":"x".repeat(32768)})),
        json!({"status":413}),
    )?;
    for media in [
        "application/octet-stream",
        "application/x-www-form-urlencoded",
    ] {
        request.send(
            format!("large={}", "x".repeat(32768)).as_bytes(),
            json!({"method":"GET","media":media}),
        )?;
    }
    request.send(
        format!("{}y=2", "x=1&".repeat(1000)).as_bytes(),
        json!({"media":"application/x-www-form-urlencoded","status":413}),
    )?;
    request.send(
        format!("deep{}=1", "[x]".repeat(33)).as_bytes(),
        json!({"media":"application/x-www-form-urlencoded","status":400}),
    )?;
    request.send(
        b"{",
        json!({"headers":{"Origin":"https://denied.invalid"},"status":403}),
    )?;
    let allowed = origin.unwrap_or("https://sub-store.vercel.app");
    request.send(
        b"{",
        json!({"method":"OPTIONS","headers":{"Origin":allowed}}),
    )?;
    for media in [
        "text/plain",
        "application/octet-stream; charset=unrecognized",
        "application/problem+json",
    ] {
        let data = request.send(b"A\0\xff", json!({"media":media}))?["data"].clone();
        json::check(
            &json!([data["0"], data["1"], data["2"]]),
            &json!([65, 0, 255]),
            false,
        )?;
    }
    let data = request.send(b"[\"array\",{\"opaque\":false}]", json!({}))?["data"].clone();
    require(
        data["0"] == "array" && data["1"] == json!({"opaque":false}),
        "Array JSON body changed",
    )?;
    for (plural, singular) in [("subs", "sub"), ("collections", "collection")] {
        let data=request.send(b"A\0\xff",json!({"path":format!("/api/{plural}"),"media":"text/plain","method":"POST","status":201}))?["data"].clone();
        json::check(
            &data,
            &if plural == "subs" {
                json!({"0":65,"1":0,"2":255})
            } else {
                json!({"type":"Buffer","data":[65,0,255]})
            },
            false,
        )?;
        let data = request.send(
            b"B\0\xff",
            json!({"path":format!("/api/{singular}/parser-{plural}"),"media":"text/plain"}),
        )?["data"]
            .clone();
        require(
            data["name"] == format!("parser-{plural}") && data["0"] == 66,
            "Raw PATCH changed name/bytes",
        )?;
        let data=request.send(b"name=form+%E4%B8%AD&subscriptions[]=one&subscriptions[]=two&custom[text]=a%2Bb%0D%0A",json!({"path":format!("/api/{plural}"),"method":"POST","status":201,"media":"application/x-www-form-urlencoded"}))?["data"].clone();
        require(
            data["name"] == "form 中"
                && data["custom"] == json!({"text":"a+b\r\n"})
                && (if plural == "subs" {
                    data.get("subscriptions").is_none()
                } else {
                    data["subscriptions"] == json!(["one", "two"])
                }),
            "Form create changed types/reserved fields",
        )?;
    }
    let forms = [
        "parserForm[a][b]=nested&parserRepeat=one&parserRepeat=two&parserEmpty&literal.dot=keep",
        "parserSparse[2]=c&parserSparse[0]=a&parserObjects[0][a]=1&parserObjects[0][b]=2",
        "parserConflict[x]=nested&parserConflict=scalar&parserNumeric[100]=object",
        "parserMalformed=%E0%A4%A&parserPlus=a+b&parserComma=a,b&parserEquals[x=y]=z",
        "__proto__[polluted]=yes&parserSafe[__proto__][polluted]=yes&constructor[prototype][opaque]=keep",
        "parserOverflow[100]=one&parserOverflow=tail",
        "parserOverflow=lead&parserOverflow[100]=one",
        "parserOverflow[100]=one&parserOverflow[2]=two&parserOverflow=tail",
        "parserOverflow[]=lead&parserOverflow[100]=one&parserOverflow[]=tail",
        "parserOverflow[100][x]=one&parserOverflow=tail",
        "parserOverflow[100]=one&parserOverflow[word]=value&parserOverflow=tail",
        "parserOverflow[word]=one&parserOverflow[100]=two&parserOverflow=tail",
        "parserOverflow=lead&parserOverflow[100][x]=one&parserOverflow[01]=two",
        "parserOverflow%5B100%5D=one&parserOverflow%5B%5D=two",
        "parserOverflow[18446744073709551615]=one&parserOverflow=tail",
        "parserOverflow[100000000000000000000]=one&parserOverflow=tail",
        "parserOverflow[1000000000000000000000]=one&parserOverflow=tail",
        "parserConflict[x]=nested&parserConflict=&parserConflict[y]=last",
        "parserConflict=first&parserConflict[x]=last&parserConflict=",
        "%EE%80%80overflow=literal&parserMarker[%EE%80%80overflow]=keep",
    ];
    let mut results = Vec::new();
    for body in forms {
        results.push(
            request.send(
                body.as_bytes(),
                json!({"media":"application/x-www-form-urlencoded"}),
            )?["data"]
                .clone(),
        );
    }
    require(
        results[0]["parserForm"] == json!({"a":{"b":"nested"}})
            && results[0]["parserRepeat"] == json!(["one", "two"])
            && results[1]["parserSparse"] == json!(["a", "c"])
            && results[1]["parserObjects"] == json!([{"a":"1","b":"2"}])
            && results[3]["parserMalformed"] == "%E0%A4%A"
            && results[3]["parserPlus"] == "a b"
            && results[4].get("polluted").is_none()
            && results[4].get("__proto__").is_none()
            && results.last().ok_or("Missing marker form")?["\u{e000}\u{e000}overflow"]
                == "literal"
            && results.last().unwrap()["parserMarker"]["\u{e000}\u{e000}overflow"] == "keep",
        "Form nesting/overflow/pollution/marker mismatch",
    )?;
    for coding in ["gzip", "deflate"] {
        let header = json!({"Content-Encoding":coding});
        let data = request.send(
            &compress(
                &raw_http::json_bytes(&json!({"parserCompressed":format!("{coding}中🙂")})),
                coding,
            )?,
            json!({"headers":{"Content-Encoding":coding.to_uppercase()}}),
        )?["data"]
            .clone();
        require(
            data["parserCompressed"] == format!("{coding}中🙂"),
            "Compressed JSON changed",
        )?;
        require(
            request.send(
                &compress(b"parserInflated[a][]=one&parserInflated[a][]=two", coding)?,
                json!({"headers":header,"media":"application/x-www-form-urlencoded"}),
            )?["data"]["parserInflated"]
                == json!({"a":["one","two"]}),
            "Compressed form changed",
        )?;
        let data = request.send(
            &compress(b"C\0\xff", coding)?,
            json!({"headers":header,"media":"application/octet-stream"}),
        )?["data"]
            .clone();
        require(
            json!([data["0"], data["1"], data["2"]]) == json!([67, 0, 255]),
            "Compressed raw bytes changed",
        )?;
        for media in [
            "application/json",
            "text/plain",
            "application/x-www-form-urlencoded",
        ] {
            request.send(
                &compress(b"", coding)?,
                json!({"headers":header,"media":media}),
            )?;
            let limit = if media == "application/json" {
                32768
            } else {
                102400
            };
            let mut exact = if media == "application/json" {
                format!("{{\"padding\":\"{}\"}}", "x".repeat(limit - 14)).into_bytes()
            } else {
                vec![b'x'; limit]
            };
            require(
                exact.len() == limit,
                "Expanded boundary fixture size changed",
            )?;
            request.send(
                &compress(&exact, coding)?,
                json!({"method":"GET","headers":header,"media":media}),
            )?;
            exact.push(b' ');
            request.send(
                &compress(&exact, coding)?,
                json!({"headers":header,"media":media,"status":413}),
            )?;
        }
        for plain in [b"{".as_slice(), b"null"] {
            request.send(
                &compress(plain, coding)?,
                json!({"headers":header,"status":400}),
            )?;
        }
        let utf16 = "{\"parserInflatedUtf16\":\"中🙂\"}"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        request.send(
            &compress(&utf16, coding)?,
            json!({"headers":header,"media":"application/json; charset=utf-16le"}),
        )?;
        request.send(&compress(b"{}",coding)?,json!({"headers":header,"media":"application/json; charset=utf-16be","status":403,"parser_error":true}))?;
        let encoded = compress(b"{}", coding)?;
        let mut corrupted = encoded.clone();
        *corrupted.last_mut().ok_or("Empty compressed fixture")? ^= 1;
        for invalid in [
            vec![],
            b"bad-header".to_vec(),
            encoded[..1].to_vec(),
            encoded[..encoded.len() - 1].to_vec(),
            corrupted,
        ] {
            request.send(&invalid, json!({"headers":header,"status":400}))?;
        }
        request.send(&encoded, json!({"headers":header,"chunk_size":1}))?;
        request.send(
            &encoded[..encoded.len() - 1],
            json!({"headers":header,"chunk_size":1,"status":400}),
        )?;
        request.send(&encoded,json!({"headers":{"Content-Encoding":coding,"Origin":"https://denied.invalid"},"status":403}))?;
        request.send(
            b"bad-header",
            json!({"headers":{"Content-Encoding":coding,"Origin":allowed},"method":"OPTIONS"}),
        )?;
        request.send(b"bad-header", json!({"headers":header,"media":null}))?;
    }
    let mut members = compress(b"{\"parserMembers\":", "gzip")?;
    members.extend(compress(b"\"all\"}", "gzip")?);
    require(
        request.send(
            &members,
            json!({"headers":{"Content-Encoding":"gzip"},"chunk_size":3}),
        )?["data"]["parserMembers"]
            == "all",
        "Concatenated gzip members lost bytes",
    )?;
    let mut empty_first = compress(b"", "gzip")?;
    empty_first.extend(compress(b"{}", "gzip")?);
    request.send(&empty_first, json!({"headers":{"Content-Encoding":"gzip"}}))?;
    let gzip = compress(b"{}", "gzip")?;
    for (suffix, status) in [
        (vec![0; 8], 200),
        (b"\0ignored".to_vec(), 200),
        (b"garbage".to_vec(), 400),
    ] {
        let mut bytes = gzip.clone();
        bytes.extend(suffix);
        request.send(
            &bytes,
            json!({"headers":{"Content-Encoding":"gzip"},"status":status}),
        )?;
    }
    let empty = compress(b"", "gzip")?;
    let mut invalid = gzip.clone();
    invalid.extend(&empty[..empty.len() - 1]);
    request.send(
        &invalid,
        json!({"headers":{"Content-Encoding":"gzip"},"status":400}),
    )?;
    let zlib = compress(b"{}", "deflate")?;
    let mut padded = zlib.clone();
    padded.extend(b"ignored");
    request.send(&padded, json!({"headers":{"Content-Encoding":"deflate"}}))?;
    request.send(
        &zlib[2..zlib.len() - 4],
        json!({"headers":{"Content-Encoding":"deflate"},"status":400}),
    )?;
    for coding in ["br", "x-gzip", "gzip, deflate"] {
        request.send(
            b"{}",
            json!({"headers":{"Content-Encoding":coding},"status":415}),
        )?;
    }
    request.send(b"{}", json!({"headers":{"Content-Encoding":""}}))?;
    media_parameters(&mut request)?;
    for plural in ["subs", "collections"] {
        let mut items = original[plural]
            .as_array()
            .ok_or("Missing original resources")?
            .clone();
        items.extend(
            api.durable()?[plural]
                .as_array()
                .ok_or("Missing parser resources")?
                .clone(),
        );
        request.send(
            &raw_http::json_bytes(&json!(items)),
            json!({"path":format!("/api/{plural}"),"method":"PUT"}),
        )?;
    }
    json::write_new(
        output.join(format!("{label}.json")),
        &json!(request.observed),
    )?;
    Ok(request.observed)
}
fn media_parameters(request: &mut Requests) -> Result {
    let json_media = [
        "Application/JSON ; charset = \"UTF-8\"",
        r#"application/json; note="a;b\"c\\d"; charset="utf\-8""#,
        r#"application/json; note="café;ok"; charset="utf-8""#,
        "application/json; charset=latin1; CHARSET=utf-8",
        "application/json; charset=latin1; charset=\"\"",
        "application/json; note=\"\"; charset=utf-8",
        "application/json+",
        "application/json; extension*=\"a=b;c\"; charset=utf-8",
    ];
    for (index, media) in json_media.iter().enumerate() {
        let value =
            json!({"label":format!("json-{index}"),"text":"参数🙂\r\n","nil":null,"flag":false});
        require(
            request.send(
                &raw_http::json_bytes(&json!({"parserMedia":value})),
                json!({"media":media,"case_id":format!("media-json-{index}")}),
            )?["data"]["parserMedia"]
                == value,
            "JSON media parameter selection changed",
        )?;
    }
    for (index, media) in [
        r#"application/x-www-form-urlencoded; charset = "UTF-8"; note="a;b""#,
        r#"application/x-www-form-urlencoded; charset="""#,
        r#"application/x-www-form-urlencoded; charset=latin1; charset="utf\-8""#,
    ]
    .iter()
    .enumerate()
    {
        let value = format!("form-{index}中🙂\r\n");
        require(
            request.send(
                format!("parserMediaForm={}", quote(&value)).as_bytes(),
                json!({"media":media,"case_id":format!("media-form-{index}")}),
            )?["data"]["parserMediaForm"]
                == value,
            "Form media parameters changed",
        )?;
    }
    for (index, media) in [
        "application/problem+json; note=\"a;b\"".into(),
        "text/plain; note=\"café\"; charset=unrecognized".into(),
        format!("{}/{}", "a".repeat(127), "b".repeat(127)),
        "application/json!; note=\"a;b\"".into(),
        "application/json+other".into(),
    ]
    .iter()
    .enumerate()
    {
        let body = [index as u8, 0, 255];
        let data = request.send(
            &body,
            json!({"media":media,"case_id":format!("media-raw-{index}")}),
        )?["data"]
            .clone();
        require(
            json!([data["0"], data["1"], data["2"]]) == json!(body),
            "Raw media selection changed bytes",
        )?;
    }
    let mut invalid = [
        "application/json;",
        "application/json; charset=",
        "application/json; note=\"unterminated",
        "application/json; \tcharset=utf-8",
        "application/json; charset\t=utf-8",
        "application/json; note=\"tab\tvalue\"",
        "application/json; charset=\"utf-8\" garbage",
        "application/json; note=\"escape\\é\"",
        "application/json; =missing",
        "application/json; note=(x)",
        "application/jso%n",
        "app.lication/json",
        "application/json+bad.suffix",
        "application/json++",
    ]
    .map(str::to_owned)
    .to_vec();
    invalid.extend([
        format!("application/{}", "a".repeat(128)),
        format!("{}/json", "a".repeat(128)),
        "application/json\t; charset=utf-8".into(),
        "application/json; charset=UTF8;".into(),
    ]);
    for (index, media) in invalid.iter().enumerate() {
        request.send(b"{",json!({"media":media,"method":"GET","headers":{"Content-Encoding":"br"},"case_id":format!("media-invalid-{index}")}))?;
    }
    for (index, charset) in [
        "utf--8",
        "utf-8_",
        "utf-8:1987",
        "utf-16-le",
        "utf-16-be",
        "utf-7-imap",
        "utf-7:1987",
    ]
    .iter()
    .enumerate()
    {
        let media = format!("application/json; charset=\"{charset}\"");
        request.send(b"{}",json!({"media":media,"status":403,"parser_error":true,"case_id":format!("charset-alias-{index}-nonempty")}))?;
        request.send(
            b"",
            json!({"media":media,"case_id":format!("charset-alias-{index}-empty")}),
        )?;
    }
    for (index, charset) in [
        "utf8",
        "ucs-2",
        "unicode-1-1-utf-8",
        "utf-32le",
        "utf-99",
        "utf-8:198",
        "utf-8:19870",
    ]
    .iter()
    .enumerate()
    {
        for (label, body) in [("nonempty", b"{}".as_slice()), ("empty", b"".as_slice())] {
            request.send(body,json!({"media":format!("application/json; charset=\"{charset}\""),"status":415,"case_id":format!("charset-unsupported-{index}-{label}")}))?;
        }
    }
    for (index, charset) in ["utf8", "utf--8", "utf-8:1987"].iter().enumerate() {
        request.send(b"{}",json!({"media":format!("application/x-www-form-urlencoded; charset=\"{charset}\""),"status":415,"case_id":format!("charset-form-{index}")}))?;
    }
    for (label, body, status) in [
        ("nonempty", compress(b"{}", "gzip")?, 403),
        ("empty", compress(b"", "gzip")?, 200),
        ("invalid", b"bad-header".to_vec(), 400),
        ("limit", compress(&vec![b'x'; 32769], "gzip")?, 413),
    ] {
        request.send(&body,json!({"media":"application/json; charset=\"utf--8\"","headers":{"Content-Encoding":"gzip"},"status":status,"parser_error":status==403,"case_id":format!("charset-gzip-{label}")}))?;
    }
    Ok(())
}
