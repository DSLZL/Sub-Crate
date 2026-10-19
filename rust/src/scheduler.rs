//! Source server jobs call the same generation, sync, backup and I/O owners as HTTP.
use crate::{
    backup, cron, json as codec, logs,
    resources::Error,
    script,
    state::{Store, truthy},
    sync,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;

#[derive(Clone)]
enum Action {
    Artifact { name: String, label: String },
    Sync,
    Produce { kind: String, name: String },
    Backup(&'static str),
    Mmdb(Vec<(String, String)>),
}

fn log(store: &Arc<Mutex<Store>>, error: bool, text: &str) {
    let mut store = store.lock().unwrap();
    if error {
        logs::error(&mut store, text);
    } else {
        logs::info(&mut store, text);
    }
}

async fn job(store: &Arc<Mutex<Store>>, operation: &str, input: Value) -> Result<(), Error> {
    script::run(store.clone(), operation, input)
        .await
        .map(|_| ())
        .map_err(|crate::conversion::Error::Legacy(message)| Error::plain(message))
}

impl Action {
    fn label(&self) -> String {
        match self {
            Self::Artifact { label, .. } => format!("ARTIFACT CRON] {label}"),
            Self::Sync => "SYNC CRON]".to_owned(),
            Self::Produce { kind, name } => format!("PRODUCE CRON] {kind} {name}"),
            Self::Backup("upload") => "UPLOAD CRON]".to_owned(),
            Self::Backup(_) => "DOWNLOAD CRON]".to_owned(),
            Self::Mmdb(_) => "MMDB CRON]".to_owned(),
        }
    }

    async fn run(&self, store: &Arc<Mutex<Store>>) -> Result<(), Error> {
        match self {
            Self::Artifact { name, .. } => sync::one(store, name).await.map(|_| ()),
            Self::Sync => sync::all(store, true).await,
            Self::Produce { kind, name } => {
                job(store, "artifact", json!({"type":kind,"name":name})).await
            }
            Self::Backup(action) => backup::action(store, &json!({"action":action})).await,
            Self::Mmdb(pairs) => {
                for (url, file) in pairs {
                    log(
                        store,
                        false,
                        &format!("[MMDB CRON] downloading {url} to {file}"),
                    );
                    if let Err(error) = job(
                        store,
                        "runtime-download-file",
                        json!({"url":url,"file":file}),
                    )
                    .await
                    {
                        log(
                            store,
                            true,
                            &format!("[MMDB CRON] {url} download failed: {}", error.message),
                        );
                    }
                }
                Ok(())
            }
        }
    }
}

fn schedule(
    store: Arc<Mutex<Store>>,
    expression: String,
    action: Action,
    running: Arc<Mutex<HashSet<String>>>,
) -> Result<JoinHandle<()>, String> {
    let masks = cron::parse(&expression)?;
    let mut due = cron::next(&masks, &jiff::Zoned::now())?;
    Ok(tokio::spawn(async move {
        loop {
            let now = jiff::Timestamp::now();
            if now < due {
                let nanos = (due.as_second() - now.as_second()) * 1_000_000_000
                    + i64::from(due.subsec_nanosecond() - now.subsec_nanosecond());
                let wait = Duration::from_nanos(nanos as u64);
                tokio::time::sleep(wait.min(Duration::from_secs(i32::MAX as u64 / 1000))).await;
                continue;
            }
            let store = store.clone();
            let running = running.clone();
            let action = action.clone();
            let label = format!("[{} {expression}", action.label());
            if let Action::Artifact { name, .. } = &action
                && !running.lock().unwrap().insert(name.clone())
            {
                log(
                    &store,
                    false,
                    &format!("{label} skipped: previous run is still running"),
                );
                match cron::next(&masks, &jiff::Zoned::now()) {
                    Ok(next) => {
                        due = next;
                        continue;
                    }
                    Err(_) => break,
                }
            }
            tokio::spawn(async move {
                log(&store, false, &format!("{label} started"));
                match action.run(&store).await {
                    Ok(()) => log(&store, false, &format!("{label} finished")),
                    Err(error) => log(&store, true, &format!("{label} error: {}", error.message)),
                }
                if let Action::Artifact { name, .. } = action {
                    running.lock().unwrap().remove(&name);
                }
            });
            match cron::next(&masks, &jiff::Zoned::now()) {
                Ok(next) => due = next,
                Err(_) => break,
            }
        }
    }))
}

struct ArtifactJobs(HashMap<String, JoinHandle<()>>);
impl Drop for ArtifactJobs {
    fn drop(&mut self) {
        for job in self.0.values() {
            job.abort();
        }
    }
}

fn artifacts(store: &Arc<Mutex<Store>>, running: &Arc<Mutex<HashSet<String>>>) -> ArtifactJobs {
    let items = store
        .lock()
        .unwrap()
        .list("artifacts")
        .unwrap_or_default()
        .to_vec();
    let mut jobs = HashMap::new();
    for item in items
        .iter()
        .filter(|item| truthy(&item["sync"]) && truthy(&item["source"]))
    {
        let expression = item
            .get("cron")
            .filter(|value| !value.is_null())
            .map(codec::js_string)
            .unwrap_or_default();
        let expression = codec::trim(&expression);
        if expression.is_empty() {
            continue;
        }
        let name = codec::js_string(&item["name"]);
        let label = sync::log_name(item);
        match schedule(
            store.clone(),
            expression.to_owned(),
            Action::Artifact {
                name: name.clone(),
                label: label.clone(),
            },
            running.clone(),
        ) {
            Ok(job) => {
                jobs.insert(name, job);
                log(
                    store,
                    false,
                    &format!("[ARTIFACT CRON] {label} {expression} scheduled"),
                );
            }
            Err(error) => log(
                store,
                true,
                &format!("[ARTIFACT CRON] {label} {expression} schedule error: {error}"),
            ),
        }
    }
    ArtifactJobs(jobs)
}

pub(crate) fn start(store: Arc<Mutex<Store>>) -> io::Result<()> {
    let running = Arc::new(Mutex::new(HashSet::new()));
    let mut updates = store.lock().unwrap().cron_refresh.subscribe();
    let jobs = artifacts(&store, &running);
    let owner = store.clone();
    let active = running.clone();
    tokio::spawn(async move {
        let mut jobs = jobs;
        while updates.changed().await.is_ok() {
            drop(jobs);
            jobs = artifacts(&owner, &active);
        }
    });
    for (variable, action) in [
        ("SUB_STORE_BACKEND_SYNC_CRON", Action::Sync),
        ("SUB_STORE_BACKEND_UPLOAD_CRON", Action::Backup("upload")),
        (
            "SUB_STORE_BACKEND_DOWNLOAD_CRON",
            Action::Backup("download"),
        ),
    ] {
        let expression = crate::configured(variable, "");
        if expression.is_empty() {
            continue;
        }
        log(
            &store,
            false,
            &format!("[{} {expression} enabled", action.label()),
        );
        schedule(store.clone(), expression, action, running.clone()).map_err(io::Error::other)?;
    }
    if crate::configured("SUB_STORE_BACKEND_SYNC_CRON", "").is_empty() {
        for variable in ["SUB_STORE_BACKEND_CRON", "SUB_STORE_CRON"] {
            if !crate::configured(variable, "").is_empty() {
                log(
                    &store,
                    true,
                    &format!("[SYNC CRON] {variable} 已弃用, 请使用 SUB_STORE_BACKEND_SYNC_CRON"),
                );
            }
        }
    }
    let produce = crate::configured("SUB_STORE_PRODUCE_CRON", "");
    if !produce.is_empty() {
        log(&store, false, &format!("[PRODUCE CRON] {produce} enabled"));
        for item in produce
            .split(';')
            .map(codec::trim)
            .filter(|item| !item.is_empty())
        {
            let mut fields = item.split(',').map(codec::trim);
            let expression = fields.next().unwrap_or_default().to_owned();
            let kind = fields.next().unwrap_or("undefined").to_owned();
            let name = fields.next().unwrap_or("undefined").to_owned();
            log(
                &store,
                false,
                &format!("[PRODUCE CRON] {kind} {name} {expression} scheduled"),
            );
            schedule(
                store.clone(),
                expression,
                Action::Produce { kind, name },
                running.clone(),
            )
            .map_err(io::Error::other)?;
        }
    }
    let pairs: Vec<_> = ["COUNTRY", "ASN"]
        .into_iter()
        .filter_map(|kind| {
            let url = crate::configured(&format!("SUB_STORE_MMDB_{kind}_URL"), "");
            let file = crate::configured(&format!("SUB_STORE_MMDB_{kind}_PATH"), "");
            (!url.is_empty() && !file.is_empty()).then_some((url, file))
        })
        .collect();
    let expression = crate::configured("SUB_STORE_MMDB_CRON", "");
    if !expression.is_empty() && !pairs.is_empty() {
        log(&store, false, &format!("[MMDB CRON] {expression} enabled"));
        schedule(store.clone(), expression, Action::Mmdb(pairs), running)
            .map_err(io::Error::other)?;
    }
    let url = crate::configured("SUB_STORE_DATA_URL", "");
    if !url.is_empty() {
        log(
            &store,
            false,
            &format!("[BACKEND] downloading data from {url}"),
        );
        tokio::spawn(async move {
            let result = async {
                let mut data = script::run(
                    store.clone(),
                    "startup-data",
                    json!({"url":url,"script":crate::configured("SUB_STORE_DATA_URL_POST", "")}),
                )
                .await
                .map_err(|crate::conversion::Error::Legacy(error)| error)?;
                let original = data.clone();
                crate::state::migrate(&mut data).map_err(|error| error.to_string())?;
                if !crate::state::valid_records(&data) {
                    return Err("Invalid state records".to_owned());
                }
                store
                    .lock()
                    .unwrap()
                    .restore(data, original)
                    .map_err(|error| error.to_string())
            }
            .await;
            match result {
                Ok(()) => log(
                    &store,
                    false,
                    &format!("[BACKEND] restored data from {url}"),
                ),
                Err(error) => {
                    log(&store, true, "[BACKEND] restore data failed");
                    eprintln!("{error}");
                    // The source rethrows its rejected startup restore promise.
                    std::process::exit(1);
                }
            }
        });
    }
    Ok(())
}
