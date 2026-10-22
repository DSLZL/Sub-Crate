use crate::{
    Result,
    http::Http,
    json, require,
    server::{Backend, Environment, Server},
};
use serde_json::{Value, json};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Api {
    pub listener: Server,
    pub http: Http,
    pub state: PathBuf,
    proof: PathBuf,
    next: Cell<usize>,
    is_reference: bool,
}
pub fn files(api: &Api) -> Result<[Vec<u8>; 2]> {
    Ok([
        std::fs::read(api.state.join("sub-store.json"))?,
        std::fs::read(api.state.join("root.json"))?,
    ])
}
impl Api {
    pub fn start(
        backend: &Backend,
        environment: &Environment,
        state: &Path,
        output: &Path,
        label: &str,
    ) -> Result<Self> {
        let proof = output.join(format!("{label}-requests"));
        std::fs::create_dir(&proof)?;
        Ok(Self {
            listener: Server::start(
                backend,
                environment,
                output,
                label,
                matches!(backend, Backend::Docker(_)),
            )?,
            http: Http::new(Duration::from_secs(5))?,
            state: state.to_owned(),
            proof,
            next: Cell::new(0),
            is_reference: matches!(backend, Backend::Node(_)),
        })
    }
    pub fn response(&self, step: &Value) -> Result<Value> {
        let response = self.http.request(&self.listener.url, step)?;
        let index = self.next.get();
        self.next.set(index + 1);
        let mut evidence = response.clone();
        evidence["body"] = json!(json::literal(
            response["body"].as_str().ok_or("Missing response body")?
        ));
        json::write_new(
            self.proof.join(format!("{index:04}.json")),
            &json!({"request":step,"response":evidence}),
        )?;
        Ok(response)
    }
    pub fn send(&self, step: &Value, status: u16) -> Result<Value> {
        let response = self.response(step)?;
        require(
            response["status"] == status,
            format!(
                "Unexpected service response: {}",
                json!({"request":step,"response":response})
            ),
        )?;
        json::parse(response["body"].as_str().ok_or("Missing response body")?)
    }
    pub fn data(&self, step: &Value) -> Result<Value> {
        Ok(self.send(step, 200)?["data"].clone())
    }
    pub fn stored(&self, file: &str) -> Result<Value> {
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            match json::read(self.state.join(file)) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let concurrent = (self.is_reference
                        && error.downcast_ref::<serde_json::Error>().is_some())
                        || (cfg!(windows)
                            && error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                                error.kind() == std::io::ErrorKind::PermissionDenied
                            }));
                    if Instant::now() >= until || !concurrent {
                        return Err(error);
                    }
                    let index = self.next.get();
                    self.next.set(index + 1);
                    json::write_new(
                        self.proof.join(format!("{index:04}-concurrent-read.json")),
                        &json!({"file":file,"error":error.to_string()}),
                    )?;
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
    pub fn durable(&self) -> Result<Value> {
        self.stored("sub-store.json")
    }
    pub fn messages(&self) -> Result<Vec<String>> {
        self.data(&json!({"path":"/api/logs?limit=1000"}))?["logs"]
            .as_array()
            .ok_or_else(|| "Missing service logs".into())
            .and_then(|entries| {
                entries
                    .iter()
                    .map(|entry| {
                        Ok(entry["message"]
                            .as_str()
                            .ok_or("Missing log message")?
                            .to_owned())
                    })
                    .collect()
            })
    }
}
pub fn wait(mut condition: impl FnMut() -> Result<bool>) -> Result {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if condition()? {
            return Ok(());
        }
        require(
            Instant::now() < deadline,
            "Actual scheduled effect timed out",
        )?;
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn file(name: &str, script: Option<&str>) -> Value {
    let mut value = json!({"name":name,"source":"local","content":"fixture content","type":"text"});
    if let Some(script) = script {
        value["process"] = json!([{"type":"Script Operator","args":{"content":script}}]);
    }
    value
}

pub fn concurrent(url: &str, steps: &[Value]) -> Result<Vec<Value>> {
    std::thread::scope(|scope| {
        let handles = steps
            .iter()
            .map(|step| scope.spawn(move || Http::new(Duration::from_secs(5))?.request(url, step)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "Concurrent HTTP worker panicked")?
            })
            .collect()
    })
}

pub fn denied_write(api: &Api, step: &Value, read: &str, file: &str, code: &str) -> Result {
    let before = api.response(&json!({"path":read}))?;
    let saved = api.state.join(format!("saved-{file}"));
    let path = api.state.join(file);
    std::fs::rename(&path, &saved)?;
    std::fs::create_dir(&path)?;
    let compare = || -> Result {
        let after = api.response(&json!({"path":read}))?;
        json::check(
            &json::parse(before["body"].as_str().ok_or("Missing pre-failure state")?)?,
            &json::parse(after["body"].as_str().ok_or("Missing post-failure state")?)?,
            false,
        )
    };
    let checked = (|| -> Result {
        let rejected = api.send(step, 500)?;
        require(
            rejected["error"]["code"] == code,
            "Unexpected write failure code",
        )?;
        if file != "root.json" {
            compare()?;
        }
        Ok(())
    })();
    std::fs::remove_dir(&path)?;
    std::fs::rename(saved, path)?;
    checked?;
    // Log reads use root.json too, so restore the fixture before checking retained entries.
    compare()
}
