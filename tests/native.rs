use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use sub_store_tests::{
    Result, contracts,
    http::{self, Http},
    json, process, reference, require,
    server::{self, Backend, Server},
};

#[test]
fn native_delivery() -> Result {
    let binary =
        PathBuf::from(std::env::var_os("TEST_RUST_BINARY").ok_or("Missing release binary")?)
            .canonicalize()?;
    sub_store_tests::distribution::native(&process::scratch("native-delivery")?, &binary)?;
    Ok(())
}

#[test]
fn settings_numeric_coercion_recipient_validation_order_and_failed_writes() -> Result {
    sub_store_tests::edges::settings_and_keys(&process::scratch("service-settings-keys")?)
}
#[test]
fn crud_typed_references_null_records_raw_buffers_restart_and_node_rollback() -> Result {
    sub_store_tests::edges::crud_lifecycle(&process::scratch("service-crud-edges")?)
}
#[test]
fn backup_import_export_codecs_calendars_atomic_restore_scalar_settings_and_node_rollback() -> Result
{
    sub_store_tests::persistence::backups(&process::scratch("service-backup-edges")?)
}
#[test]
fn noisy_base64_legacy_migration_preserves_numeric_order_processors_and_original_snapshot() -> Result
{
    sub_store_tests::persistence::migration(&process::scratch("service-migration")?)
}
#[test]
fn unicode_json_and_utf16_keep_lone_surrogates_markers_restart_and_node_rollback() -> Result {
    sub_store_tests::unicode::run(&process::scratch("service-unicode")?)
}
#[test]
fn live_raw_buffers_keep_identity_across_failed_writes_restore_restart_and_node_rollback() -> Result {
    sub_store_tests::buffers::run(&process::scratch("service-buffers")?)
}
#[test]
fn runtime_proxy_header_limits_listener_prefix_cors_and_json_limits_preserve_contracts() -> Result {
    sub_store_tests::configuration::run(&process::scratch("service-configuration")?)
}
#[test]
fn diagnostic_envelopes_native_frames_reasons_and_failed_state_match_all_environment_modes() -> Result {
    sub_store_tests::configuration::diagnostics(&process::scratch("service-diagnostics")?)
}
#[test]
fn corrupt_native_startups_and_restore_journals_exit_without_resetting_either_file() -> Result {
    sub_store_tests::configuration::rejected_startups(&process::scratch("service-rejected-startups")?)
}
#[test]
fn merged_and_standalone_static_ranges_validators_api_methods_queries_and_forwarded_bodies() -> Result {
    sub_store_tests::static_files::run(&process::scratch("service-static-http")?)
}

#[test]
fn frozen_node_unit_suite() -> Result {
    reference::unit(&process::scratch("reference-unit")?)
}

#[test]
fn frozen_node_http_suites() -> Result {
    reference::http_suites(&process::scratch("reference-http")?)
}

#[test]
fn shared_native_script_host_processors_dns_mmdb_wire_state_and_resume() -> Result {
    sub_store_tests::probe::shared(&process::scratch("script-probe")?)
}

#[test]
fn fresh_age_processes_cross_decrypt_both_key_types_and_preserve_artifacts() -> Result {
    let binary = PathBuf::from(
        std::env::var_os("TEST_PROBE_BINARY").ok_or("Missing actual Rust probe executable")?,
    )
    .canonicalize()?;
    sub_store_tests::age::run(&process::scratch("age-probe")?, &binary)
}

#[test]
fn downloads_preserve_exact_wire_cache_stream_bytes_files_and_resume() -> Result {
    let binary = PathBuf::from(
        std::env::var_os("TEST_PROBE_BINARY").ok_or("Missing actual Rust probe executable")?,
    )
    .canonicalize()?;
    sub_store_tests::download::run(&process::scratch("download-probe")?, &binary)
}

#[test]
fn conversion_business_contracts_clocks_restarts_isolation_and_concurrency() -> Result {
    sub_store_tests::service::conversions(&process::scratch("service-conversions")?)
}

#[test]
fn services_cross_decrypt_existing_age_backups_and_rollback_corruption() -> Result {
    sub_store_tests::backup::age_interop(&process::scratch("service-backup-age")?)
}

#[test]
fn actual_schedules_cover_overlap_updates_disable_delete_sync_backup_mmdb_and_startup() -> Result {
    sub_store_tests::scheduling::run(&process::scratch("service-scheduling")?)
}

#[test]
fn actual_utilities_and_sync_preserve_wire_cache_ttl_state_and_rf02() -> Result {
    sub_store_tests::business::utilities_sync(&process::scratch("service-utilities-sync")?)
}
#[test]
fn archive_lifecycle_preserves_restart_atomic_restore_and_write_failures() -> Result {
    sub_store_tests::business::archives(&process::scratch("service-archives")?)
}
#[test]
fn logs_preserve_typed_seed_capture_filter_retention_restart_and_failed_clear() -> Result {
    sub_store_tests::business::logs(&process::scratch("service-logs")?)
}
#[test]
fn notifications_preserve_provider_templates_get_bodies_parallel_delivery_and_recovery() -> Result {
    sub_store_tests::notifications::run(&process::scratch("service-notifications")?)
}
#[test]
fn business_resource_sync_settings_and_backup_log_callers_are_exercised() -> Result {
    sub_store_tests::business::log_callers(&process::scratch("service-business-logs")?)
}
#[test]
fn merged_and_standalone_shares_preserve_token_concurrency_encryption_and_failed_writes() -> Result
{
    sub_store_tests::sharing::run(&process::scratch("service-sharing")?)
}

#[test]
fn rust_and_node_preserve_shared_state_restart_write_failure_and_rollback() -> Result {
    let output = process::scratch("state-parity")?;
    reference::verify(&output)?;
    let binary =
        PathBuf::from(std::env::var_os("TEST_RUST_BINARY").ok_or("Missing delivered Rust binary")?)
            .canonicalize()?;
    let http = Http::new(Duration::from_secs(5))?;
    let cases = contracts::cases("service-state")?;
    let restart = contracts::cases("restart")?;
    let mut final_states = Vec::new();
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", Backend::Rust(binary.clone())),
    ] {
        let state = output.join(format!("{kind}-state"));
        std::fs::create_dir(&state)?;
        let mut environment = server::environment(&state);
        server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
        server::set(
            &mut environment,
            "SUB_STORE_FRONTEND_BACKEND_PATH",
            "/backend",
        );
        {
            let _listener = Server::start(&backend, &environment, &output, kind, false)?;
            contracts::check(
                &http,
                &_listener.url,
                &state,
                &cases,
                None,
                &output.join(format!("{kind}-contracts.json")),
            )?;
            final_states.push(json::read(state.join("sub-store.json"))?);
        }
        {
            let label = format!("{kind}-restart");
            let _listener = Server::start(&backend, &environment, &output, &label, false)?;
            contracts::check(
                &http,
                &_listener.url,
                &state,
                &restart,
                None,
                &output.join(format!("{kind}-restart.json")),
            )?;
            if kind == "rust" {
                let previous = http.request(&_listener.url, &json!({"path":"/api/settings"}))?;
                let saved = state.join("saved-state.json");
                let current = state.join("sub-store.json");
                std::fs::rename(&current, &saved)?;
                std::fs::create_dir(&current)?;
                let result = (|| -> Result {
                    let failed=http.request(&_listener.url,&json!({"path":"/api/settings","method":"PATCH","body":{"mustNotCommit":true}}))?;
                    let unchanged =
                        http.request(&_listener.url, &json!({"path":"/api/settings"}))?;
                    json::write_new(
                        output.join("write-failure.json"),
                        &json!({"before":previous,"failure":failed,"after":unchanged}),
                    )?;
                    require(
                        failed["status"] == 500
                            && json::parse(failed["body"].as_str().unwrap())?["error"]["code"]
                                == "STATE_WRITE_FAILED",
                        "Failed state writes must return STATE_WRITE_FAILED",
                    )?;
                    json::check(
                        &json::parse(unchanged["body"].as_str().unwrap())?,
                        &json::parse(previous["body"].as_str().unwrap())?,
                        false,
                    )
                })();
                std::fs::remove_dir(&current)?;
                std::fs::rename(&saved, &current)?;
                result?;
                let denied=http.request(&_listener.url,&json!({"path":"/api/settings","method":"PATCH","headers":{"Origin":"https://denied.invalid"},"body":{"mustNotCommit":true}}))?;
                http::check(
                    &denied,
                    &json!({"status":403,"absent_headers":["access-control-allow-origin"]}),
                )?;
                require(
                    http.request(
                        &_listener.url,
                        &json!({"path":"/api/storage","origin":true}),
                    )?["status"]
                        == 404,
                    "Unprefixed API must be unavailable",
                )?;
                require(http.request(&_listener.url,&json!({"path":"/api/settings","method":"OPTIONS","headers":{"Origin":"https://sub-store.vercel.app"}}))?["status"]==200,"CORS preflight failed")?;
            }
        }
    }
    json::check(&final_states[0], &final_states[1], false)?;
    let rollback = output.join("node-state");
    std::fs::copy(
        output.join("rust-state/sub-store.json"),
        rollback.join("sub-store.json"),
    )?;
    let mut environment = server::environment(&rollback);
    server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
    server::set(
        &mut environment,
        "SUB_STORE_FRONTEND_BACKEND_PATH",
        "/backend",
    );
    let _listener = Server::start(
        &Backend::Node(reference::node()),
        &environment,
        &output,
        "rollback",
        false,
    )?;
    contracts::check(
        &http,
        &_listener.url,
        &rollback,
        &restart,
        None,
        &output.join("rollback.json"),
    )?;
    Ok(())
}
