//! Public Gist/Snippet script helper over the existing native HTTP host.
use crate::{
    js_language::Js,
    script, script_network as network,
    script_pipeline::{object, optional},
};
use rquickjs::{
    Ctx, Function, Object, Value,
    prelude::{Async, Func, Opt, This},
};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/81.0.4044.141 Safari/537.36";

fn keys<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Vec<Value<'js>>> {
    let constructor = js.ctx.globals().get("Object")?;
    js.call("iterable", (js.method(constructor, "keys", vec![value])?,))
}

fn get<'js>(js: &Js<'js>, value: Value<'js>, key: Value<'js>) -> rquickjs::Result<Value<'js>> {
    js.call("get", (value, key))
}
fn set<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    key: Value<'js>,
    field: Value<'js>,
) -> rquickjs::Result<()> {
    js.call("set", (value, key, field))
}
fn remove<'js>(js: &Js<'js>, value: Value<'js>, key: Value<'js>) -> rquickjs::Result<()> {
    js.call("remove", (value, key))
}
fn reject(js: &Js<'_>, message: &str) -> rquickjs::Error {
    match js.value(message) {
        Ok(value) => js.ctx.throw(value),
        Err(error) => error,
    }
}
fn nullish(value: &Value<'_>) -> bool {
    value.is_null() || value.is_undefined()
}

fn response<'js>(ctx: Ctx<'js>, response: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let status = js.text(js.get(&response, "statusCode")?)?;
    if !status.starts_with(['4', '5']) {
        return Ok(response);
    }
    let body = js.get(&response, "body")?;
    let parsed = match js.call::<Value, _>("jsonParse", (body.clone(),)) {
        Ok(value) => value,
        Err(_) => {
            js.ctx.catch();
            js.undefined()
        }
    };
    let message = optional(&js, &parsed, "message")?;
    let mut detail = js.undefined();
    for candidate in [
        optional(&js, &message, "error")?,
        optional(&js, &parsed, "error")?,
        message,
        body,
        js.value("Unknown error")?,
    ] {
        if !nullish(&candidate) {
            detail = candidate;
            break;
        }
    }
    Err(reject(
        &js,
        &format!("ERROR: HTTP {status}: {}", js.text(detail)?),
    ))
}

fn initialize_gist<'js>(
    ctx: Ctx<'js>,
    this: Value<'js>,
    options: Value<'js>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let token = js.text(js.get(&options, "token")?)?;
    let key = js.get(&options, "key")?;
    let platform = js.get(&options, "syncPlatform")?;
    let gitlab = js.equal(&platform, &js.value("gitlab")?)?;
    let settings = network::settings(&js)?;
    let timeout = network::prefer(
        &js,
        [js.get(&settings, "githubApiTimeout")?, js.value(10000)?],
    )?;
    let configured = js.get(&settings, "githubApiUrl")?;
    let raw = if nullish(&configured) {
        String::new()
    } else {
        js.text(configured)?
    };
    let raw = crate::json::trim(&raw);
    let github = if raw.is_empty() {
        "https://api.github.com"
    } else {
        raw
    }
    .trim_end_matches('/');
    let proxy = network::prefer(&js, [js.get(&settings, "githubProxy")?, js.value("")?])?;
    let proxy = js.text(proxy)?;
    let proxy = crate::json::trim(&proxy).trim_end_matches('/');
    let base = if gitlab {
        "https://gitlab.com/api/v4".to_owned()
    } else if github != "https://api.github.com" || proxy.is_empty() {
        github.to_owned()
    } else {
        format!("{proxy}/{github}")
    };
    let headers = object(&js)?;
    js.set(
        &headers,
        if gitlab {
            "PRIVATE-TOKEN"
        } else {
            "Authorization"
        },
        js.value(if gitlab {
            token
        } else {
            format!("token {token}")
        })?,
    )?;
    js.set(&headers, "User-Agent", js.value(USER_AGENT)?)?;
    let defaults = object(&js)?;
    js.set(&defaults, "baseURL", js.value(base)?)?;
    js.set(
        &defaults,
        "headers",
        js.call("spread", (object(&js)?, headers.clone()))?,
    )?;
    let default_proxy = network::prefer(
        &js,
        [
            js.get(&settings, "defaultProxy")?,
            js.value(std::env::var("SUB_STORE_BACKEND_DEFAULT_PROXY").unwrap_or_default())?,
        ],
    )?;
    if js.truthy(&default_proxy)? {
        js.set(&defaults, "proxy", default_proxy)?;
    }
    js.set(&defaults, "timeout", timeout)?;
    let events = object(&js)?;
    js.set(
        &events,
        "onResponse",
        Function::new(js.ctx.clone(), response)?.into_value(),
    )?;
    js.set(&defaults, "events", events)?;
    let factory: Function = js.ctx.globals().get("createHostHTTP")?;
    let http: Value = factory.call((defaults,))?;
    js.set(&this, "headers", headers)?;
    js.set(&this, "http", http.clone())?;
    js.set(&this, "key", key)?;
    js.set(&this, "syncPlatform", platform)?;
    let queue:Function=js.ctx.eval(r#"(owner,http) => {
      owner._requestQueue=Promise.resolve();
      for(const method of ['get','post','put','patch','delete']) {
        const original=http[method];
        if(typeof original==='function')http[method]=(...args)=>new Promise((resolve,reject)=>{
          owner._requestQueue=owner._requestQueue.then(()=>original.apply(http,args).then(resolve,reject));
        });
      }
    }"#)?;
    queue.call((this, http))
}

async fn send<'js>(
    js: &Js<'js>,
    this: Value<'js>,
    method: &str,
    input: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    script::settled(js, js.method(js.get(&this, "http")?, method, vec![input])?).await
}

async fn locate<'js>(ctx: Ctx<'js>, This(this): This<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let gitlab = js.equal(&js.get(&this, "syncPlatform")?, &js.value("gitlab")?)?;
    let response = send(
        &js,
        this.clone(),
        "get",
        js.value(if gitlab {
            "/snippets"
        } else {
            "/gists?per_page=100&page=1"
        })?,
    )
    .await?;
    let gists: Value = js.call("jsonParse", (js.get(&response, "body")?,))?;
    let gists: Vec<Value> = js.call("iterable", (gists,))?;
    for gist in gists {
        if js.equal(
            &js.get(&gist, if gitlab { "title" } else { "description" })?,
            &js.get(&this, "key")?,
        )? {
            return Ok(gist);
        }
    }
    Ok(js.undefined())
}

fn action<'js>(
    js: &Js<'js>,
    kind: &str,
    key: Value<'js>,
    content: Option<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let action = object(js)?;
    js.set(&action, "action", js.value(kind)?)?;
    js.set(&action, "file_path", key)?;
    if let Some(content) = content {
        js.set(&action, "content", content)?;
    }
    Ok(action)
}

async fn upload<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    input: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let input = input.0.unwrap_or_else(|| js.undefined());
    if keys(&js, input.clone())?.is_empty() {
        return Err(reject(&js, "未提供需上传的文件"));
    }
    let gist = script::settled(&js, js.method(this.clone(), "locate", vec![])?).await?;
    let options = options
        .0
        .filter(|value| !value.is_undefined())
        .unwrap_or(object(&js)?);
    let fallback = js.get(&options, "emptyFileFallback")?;
    let filename = optional(&js, &fallback, "filename")?;
    let existing = js.truthy(&optional(&js, &gist, "id")?)?;
    let gitlab = js.equal(&js.get(&this, "syncPlatform")?, &js.value("gitlab")?)?;
    let mut files = input;
    let mut actions = Vec::new();
    let mut fallback_status = None;
    if existing {
        if gitlab {
            let old = js.get(&gist, "files")?;
            let old: Vec<Value> = js.call("iterable", (old,))?;
            let map = object(&js)?;
            for file in old {
                set(&js, map.clone(), js.get(&file, "path")?, file)?;
            }
            js.set(&gist, "files", map)?;
        }
        let previous = js.get(&gist, "files")?;
        let result: Value = js.call("spread", (object(&js)?, previous.clone()))?;
        for key in keys(&js, files.clone())? {
            let file = get(&js, files.clone(), key.clone())?;
            let content = js.get(&file, "content")?;
            let old = js.truthy(&get(&js, result.clone(), key.clone())?)?;
            if nullish(&content) || js.equal(&content, &js.value("")?)? {
                remove(&js, result.clone(), key.clone())?;
                if old {
                    set(
                        &js,
                        files.clone(),
                        key.clone(),
                        Value::new_null(js.ctx.clone()),
                    )?;
                    actions.push(action(&js, "delete", key, None)?);
                } else {
                    remove(&js, files.clone(), key)?;
                }
            } else {
                set(&js, result.clone(), key.clone(), file)?;
                actions.push(action(
                    &js,
                    if old { "update" } else { "create" },
                    key,
                    Some(content),
                )?);
            }
        }
        if js.truthy(&filename)? {
            let count = keys(&js, result.clone())?.len();
            let placeholder = js.truthy(&get(&js, result.clone(), filename.clone())?)?;
            let mut real = false;
            for key in keys(&js, result.clone())? {
                if !js.equal(&key, &filename)? {
                    real = true;
                    break;
                }
            }
            if count == 0 {
                let content = js.get(&fallback, "content")?;
                let content = if nullish(&content) {
                    js.value("")?
                } else {
                    content
                };
                let file = object(&js)?;
                js.set(&file, "content", content.clone())?;
                set(&js, result.clone(), filename.clone(), file.clone())?;
                fallback_status = Some("created");
                if gitlab {
                    let old = js.truthy(&get(&js, previous.clone(), filename.clone())?)?;
                    actions.push(action(
                        &js,
                        if old { "update" } else { "create" },
                        filename.clone(),
                        Some(content),
                    )?);
                } else {
                    set(&js, files.clone(), filename.clone(), file)?;
                }
            } else if placeholder && real {
                remove(&js, result.clone(), filename.clone())?;
                fallback_status = Some("removed");
                if gitlab {
                    actions.push(action(&js, "delete", filename.clone(), None)?);
                } else {
                    set(
                        &js,
                        files.clone(),
                        filename.clone(),
                        Value::new_null(js.ctx.clone()),
                    )?;
                }
            } else if placeholder {
                fallback_status = Some("retained");
            }
        }
        let count = keys(&js, result)?.len();
        if count == 0 {
            return Err(reject(
                &js,
                if gitlab {
                    "本次操作将导致所有文件的内容都为空, 无法更新 snippet"
                } else {
                    "本次操作将导致所有文件的内容都为空, 无法更新 gist"
                },
            ));
        }
        if gitlab && count > 10 {
            return Err(reject(
                &js,
                "本次操作将导致 snippet 的文件数超过 10, 无法更新 snippet",
            ));
        }
        if gitlab {
            files = js.array(actions)?;
        }
    } else {
        let filtered = object(&js)?;
        for key in keys(&js, files.clone())? {
            let file = get(&js, files.clone(), key.clone())?;
            let content = js.get(&file, "content")?;
            if !content.is_null() && !js.equal(&content, &js.value("")?)? {
                set(&js, filtered.clone(), key, file)?;
            }
        }
        files = filtered;
        if keys(&js, files.clone())?.is_empty() {
            return Err(reject(
                &js,
                if gitlab {
                    "所有文件的内容都为空, 无法创建 snippet"
                } else {
                    "所有文件的内容都为空, 无法创建 gist"
                },
            ));
        }
        if gitlab {
            let mut list = Vec::new();
            for key in keys(&js, files.clone())? {
                let file = object(&js)?;
                js.set(&file, "file_path", key.clone())?;
                js.set(
                    &file,
                    "content",
                    js.get(&get(&js, files.clone(), key)?, "content")?,
                )?;
                list.push(file);
            }
            files = js.array(list)?;
        }
    }
    let body = object(&js)?;
    if !existing {
        js.set(
            &body,
            if gitlab { "title" } else { "description" },
            js.get(&this, "key")?,
        )?;
        js.set(
            &body,
            if gitlab { "visibility" } else { "public" },
            if gitlab {
                js.value("private")?
            } else {
                js.value(false)?
            },
        )?;
    }
    js.set(&body, "files", files)?;
    let request = object(&js)?;
    let mut url = if gitlab { "/snippets" } else { "/gists" }.to_owned();
    if existing {
        url.push('/');
        url.push_str(&js.text(js.get(&gist, "id")?)?);
    }
    js.set(&request, "url", js.value(url)?)?;
    if gitlab || existing {
        let headers: Value = js.call("spread", (object(&js)?, js.get(&this, "headers")?))?;
        js.set(
            &headers,
            if gitlab {
                "Content-Type"
            } else {
                "Accept-Encoding"
            },
            js.value(if gitlab {
                "application/json"
            } else {
                "gzip, deflate, br"
            })?,
        )?;
        js.set(&request, "headers", headers)?;
    }
    js.set(
        &request,
        "body",
        crate::js_values::stringify(&js.ctx, body, 0)?,
    )?;
    let method = if existing {
        if gitlab { "put" } else { "patch" }
    } else {
        "post"
    };
    let response = send(&js, this, method, request).await?;
    if let Some(status) = fallback_status {
        let detail = object(&js)?;
        js.set(&detail, "status", js.value(status)?)?;
        js.set(&detail, "filename", filename)?;
        let meta = object(&js)?;
        js.set(&meta, "emptyFileFallback", detail)?;
        js.set(&response, "subStoreUploadMeta", meta)?;
    }
    Ok(response)
}

async fn download<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    filename: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let gist = script::settled(&js, js.method(this.clone(), "locate", vec![])?).await?;
    let id = optional(&js, &gist, "id")?;
    if !js.truthy(&id)? {
        return Err(reject(
            &js,
            &format!(
                "找不到 Sub-Store Gist ({})",
                js.text(js.get(&this, "key")?)?
            ),
        ));
    }
    let response = send(
        &js,
        this.clone(),
        "get",
        js.value(format!("/gists/{}", js.text(id)?))?,
    )
    .await?;
    let data: Value = js.call("jsonParse", (js.get(&response, "body")?,))?;
    let file = get(
        &js,
        js.get(&data, "files")?,
        filename.0.unwrap_or_else(|| js.undefined()),
    )?;
    let response = send(&js, this, "get", js.get(&file, "raw_url")?).await?;
    js.get(&response, "body")
}

pub(crate) async fn artifact_upload<'js>(
    js: &Js<'js>,
    files: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let settings = network::settings(js)?;
    let token = js.get(&settings, "gistToken")?;
    if !js.truthy(&token)? {
        return Err(reject(js, "未设置 GitHub Token！"));
    }
    let options = object(js)?;
    js.set(&options, "token", token)?;
    js.set(&options, "syncPlatform", js.get(&settings, "syncPlatform")?)?;
    js.set(&options, "key", js.value("Sub-Store Artifacts Repository")?)?;
    let manager = object(js)?;
    initialize_gist(js.ctx.clone(), manager.clone(), options)?;
    js.set(
        &manager,
        "locate",
        Function::new(js.ctx.clone(), Async(locate))?.into_value(),
    )?;
    let fallback = object(js)?;
    js.set(&fallback, "filename", js.value(".sub-store-placeholder")?)?;
    js.set(&fallback, "content", js.value("Sub-Store placeholder\nThis file keeps the Gist alive when all sync configuration files are deleted.")?)?;
    let options = object(js)?;
    js.set(&options, "emptyFileFallback", fallback)?;
    upload(
        js.ctx.clone(),
        This(manager),
        Opt(Some(files)),
        Opt(Some(options)),
    )
    .await
}

pub(crate) fn initialize<'js>(ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    ctx.globals()
        .set("__initGist", Func::from(initialize_gist))?;
    let constructor: Function =
        ctx.eval("(class Gist {constructor(options){__initGist(this,options);}})")?;
    let prototype: Object = constructor.get("prototype")?;
    prototype.set("locate", Func::from(Async(locate)))?;
    prototype.set("upload", Func::from(Async(upload)))?;
    prototype.set("download", Func::from(Async(download)))?;
    let descriptors:Function=ctx.eval("prototype => {for(const key of ['locate','upload','download'])Object.defineProperty(prototype,key,{enumerable:false});}")?;
    descriptors.call::<_, ()>((prototype,))?;
    utils.set("Gist", constructor)
}
