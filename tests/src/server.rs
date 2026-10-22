use crate::{
    Result,
    http::Http,
    json,
    process::{self, Running},
    require,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub type Environment = BTreeMap<OsString, OsString>;

pub fn environment(state: &Path) -> Environment {
    let mut environment: Environment = std::env::vars_os()
        .filter(|(key, _)| !key.to_string_lossy().starts_with("SUB_STORE_"))
        .collect();
    for (key, value) in [
        ("SUB_STORE_DATA_BASE_PATH", state.as_os_str()),
        ("SUB_STORE_FRONTEND_BACKEND_PATH", "/".as_ref()),
        ("SUB_STORE_BACKEND_API_HOST", "127.0.0.1".as_ref()),
    ] {
        environment.insert(key.into(), value.to_owned());
    }
    environment
}

pub fn set(environment: &mut Environment, key: &str, value: impl AsRef<std::ffi::OsStr>) {
    environment.insert(key.into(), value.as_ref().to_owned());
}
fn get<'a>(environment: &'a Environment, key: &str) -> Option<&'a str> {
    environment
        .get(std::ffi::OsStr::new(key))
        .and_then(|value| value.to_str())
}
fn truth(environment: &Environment, key: &str) -> bool {
    get(environment, key).is_some_and(|value| !value.is_empty())
}
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}
fn docker_user() -> Result<String> {
    #[cfg(unix)]
    {
        Ok(format!("{}:{}", unsafe { libc::getuid() }, unsafe {
            libc::getgid()
        }))
    }
    #[cfg(not(unix))]
    {
        Err("Docker acceptance requires a native Linux runner".into())
    }
}

#[derive(Clone)]
pub enum Backend {
    Node(PathBuf),
    Rust(PathBuf),
    Docker(String),
}

pub struct Server {
    pub process: Running,
    pub url: String,
    pub origin: String,
    pub frontend: String,
    output: PathBuf,
    label: String,
    container: Option<String>,
}

impl Server {
    pub fn start(
        backend: &Backend,
        environment: &Environment,
        output: &Path,
        label: &str,
        host_network: bool,
    ) -> Result<Self> {
        let mut environment = environment.clone();
        let port =
            match get(&environment, "SUB_STORE_BACKEND_API_PORT").filter(|value| *value != "0") {
                Some(port) => port.parse()?,
                None => free_port()?,
            };
        set(
            &mut environment,
            "SUB_STORE_BACKEND_API_PORT",
            port.to_string(),
        );
        let origin = format!("http://127.0.0.1:{port}");
        if truth(&environment, "SUB_STORE_FRONTEND_PATH")
            && !environment.contains_key(std::ffi::OsStr::new("SUB_STORE_CORS_ALLOWED_ORIGINS"))
        {
            set(&mut environment, "SUB_STORE_CORS_ALLOWED_ORIGINS", &origin);
        }
        let url = format!(
            "{}{}",
            origin,
            if truth(&environment, "SUB_STORE_BACKEND_MERGE")
                || truth(&environment, "SUB_STORE_BACKEND_PREFIX")
            {
                get(&environment, "SUB_STORE_FRONTEND_BACKEND_PATH")
                    .unwrap_or("")
                    .trim_end_matches('/')
            } else {
                ""
            }
        );
        let separate = truth(&environment, "SUB_STORE_FRONTEND_PATH")
            && !truth(&environment, "SUB_STORE_BACKEND_MERGE");
        let frontend = if separate {
            format!(
                "http://127.0.0.1:{}",
                get(&environment, "SUB_STORE_FRONTEND_PORT").unwrap_or("3001")
            )
        } else {
            origin.clone()
        };
        let mut container = None;
        let mut command = match backend {
            Backend::Node(node) => {
                let mut command = Command::new(node);
                command
                    .arg(crate::root().join("backend/dist/sub-store.bundle.js"))
                    .current_dir(crate::root().join("backend"));
                command
            }
            Backend::Rust(binary) => {
                set(&mut environment, "PATH", "");
                let mut command = Command::new(binary);
                command.current_dir(crate::root());
                command
            }
            Backend::Docker(image) => {
                let name = format!("nextest-{}-{port}", std::process::id());
                let mut command = Command::new("docker");
                command.args([
                    "run",
                    "--rm",
                    "--name",
                    &name,
                    "--read-only",
                    "--cap-drop=ALL",
                    "--security-opt=no-new-privileges",
                    "--user",
                ]);
                command.arg(docker_user()?);
                command.args([
                    "--mount",
                    &format!(
                        "type=bind,source={},target=/data",
                        get(&environment, "SUB_STORE_DATA_BASE_PATH")
                            .ok_or("Missing Docker state")?
                    ),
                ]);
                if host_network {
                    command.arg("--network=host");
                } else {
                    command.args(["-p", &format!("127.0.0.1:{port}:3000")]);
                }
                if let Some(frontend) = get(&environment, "SUB_STORE_FRONTEND_PATH") {
                    command.args([
                        "--mount",
                        &format!("type=bind,source={frontend},target=/frontend,readonly"),
                    ]);
                    if separate && !host_network {
                        command.args([
                            "-p",
                            &format!(
                                "127.0.0.1:{}:3001",
                                get(&environment, "SUB_STORE_FRONTEND_PORT").unwrap_or("3001")
                            ),
                        ]);
                    }
                }
                let database = Path::new(
                    get(&environment, "SUB_STORE_DATA_BASE_PATH").ok_or("Missing Docker state")?,
                );
                if ["SUB_STORE_MMDB_COUNTRY_PATH", "SUB_STORE_MMDB_ASN_PATH"]
                    .iter()
                    .filter_map(|key| get(&environment, key))
                    .any(|path| !Path::new(path).starts_with(database))
                {
                    command.args([
                        "--mount",
                        &format!(
                            "type=bind,source={},target=/mmdb,readonly",
                            crate::compat().join("fixtures/mmdb").display()
                        ),
                    ]);
                }
                for (key, value) in &environment {
                    let key = key.to_str().ok_or("Invalid Docker environment key")?;
                    let value = value.to_str().ok_or("Invalid Docker environment value")?;
                    if key.starts_with("SUB_STORE_")
                        || [
                            "TZ",
                            "NODE_ENV",
                            "HTTP_PROXY",
                            "http_proxy",
                            "HTTPS_PROXY",
                            "https_proxy",
                            "ALL_PROXY",
                            "all_proxy",
                            "NO_PROXY",
                            "no_proxy",
                        ]
                        .contains(&key)
                    {
                        let value = match key {
                            "SUB_STORE_DATA_BASE_PATH" => "/data".to_owned(),
                            "SUB_STORE_BACKEND_API_PORT" => {
                                if host_network {
                                    port.to_string()
                                } else {
                                    "3000".to_owned()
                                }
                            }
                            "SUB_STORE_BACKEND_API_HOST" | "SUB_STORE_FRONTEND_HOST" => {
                                if host_network {
                                    value.to_owned()
                                } else {
                                    "0.0.0.0".to_owned()
                                }
                            }
                            "SUB_STORE_FRONTEND_PATH" => "/frontend".to_owned(),
                            "SUB_STORE_FRONTEND_PORT" => {
                                if host_network {
                                    value.to_owned()
                                } else {
                                    "3001".to_owned()
                                }
                            }
                            "SUB_STORE_MMDB_COUNTRY_PATH" | "SUB_STORE_MMDB_ASN_PATH" => {
                                match Path::new(value).strip_prefix(database) {
                                    Ok(relative) => format!(
                                        "/data/{}",
                                        relative.to_string_lossy().replace('\\', "/")
                                    ),
                                    Err(_) => format!(
                                        "/mmdb/{}",
                                        Path::new(value)
                                            .file_name()
                                            .ok_or("Invalid MMDB path")?
                                            .to_string_lossy()
                                    ),
                                }
                            }
                            _ => value.to_owned(),
                        };
                        command.args(["--env", &format!("{key}={value}")]);
                    }
                }
                command.arg(image);
                container = Some(name);
                command
            }
        };
        command.env_clear().envs(environment);
        let argv: Vec<_> = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        let process = Running::spawn(&mut command, output, label)?;
        let mut server = Self {
            process,
            url,
            origin,
            frontend,
            output: output.to_owned(),
            label: label.to_owned(),
            container,
        };
        json::write_new(
            output.join(format!("{label}.listener.json")),
            &json!({"argv":argv,"pid":server.process.child.id(),"url":server.url,"origin":server.origin,"rtk_exception":"Direct listener ownership is required for Windows cleanup and native exit status"}),
        )?;
        let http = Http::new(Duration::from_secs(5))?;
        let started = Instant::now();
        loop {
            require(
                server.process.child.try_wait()?.is_none(),
                "Listener exited before readiness; inspect preserved logs",
            )?;
            let ready = http
                .request(&server.url, &json!({"path":"/api/utils/env"}))
                .is_ok_and(|response| response["status"] == 200);
            let frontend_ready = !separate
                || http
                    .request(&server.frontend, &json!({"path":"/"}))
                    .is_ok_and(|response| response["status"] == 200);
            if ready && frontend_ready {
                break;
            }
            require(
                started.elapsed() < Duration::from_secs(30),
                "Listener readiness timeout",
            )?;
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(server)
    }
    pub fn stop(&mut self) -> Result {
        if let Some(name) = self.container.take() {
            process::checked(
                Command::new("docker").args(["rm", "--force", &name]),
                &self.output,
                &format!("{}-remove", self.label),
                Duration::from_secs(30),
            )?;
        }
        let status = self.process.stop()?;
        let path = self.output.join(format!("{}.exit.json", self.label));
        if !path.exists() {
            json::write_new(path, &json!({"exit_code":status.code()}))?;
        }
        Ok(())
    }
    pub fn metadata(&self) -> Value {
        json!({"url":self.url,"origin":self.origin,"pid":self.process.child.id()})
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
