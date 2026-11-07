use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};
use sub_crate_tests::{
    Result,
    http::{self, Http},
    json, process, require, server,
};
mod docker {
    use super::*;
    #[test]
    fn compose_example_loads_environment_and_preserves_data_after_recreation() -> Result {
        let output = process::scratch("docker-compose")?;
        let image = std::env::var("TEST_DOCKER_IMAGE")?;
        let port = server::free_port()?.to_string();
        let project = format!("nextest-compose-{}", std::process::id());
        fs::copy(
            sub_crate_tests::root().join("docker-compose.example.yml"),
            output.join("docker-compose.example.yml"),
        )?;
        let environment = fs::read_to_string(sub_crate_tests::root().join(".env.example"))?;
        fs::write(
            output.join(".env"),
            format!("{environment}\nSUB_STORE_X_POWERED_BY=compose-env-fixture\n"),
        )?;
        let execute = |arguments: &[&str], label: &str| {
            process::checked(
                Command::new("docker")
                    .current_dir(&output)
                    .args([
                        "compose",
                        "--project-name",
                        &project,
                        "--file",
                        "docker-compose.example.yml",
                    ])
                    .args(arguments)
                    .env("SUB_CRATE_IMAGE", &image)
                    .env("SUB_CRATE_BIND", "127.0.0.1")
                    .env("SUB_CRATE_PORT", &port),
                &output,
                label,
                Duration::from_secs(90),
            )
        };
        let result = (|| -> Result {
            execute(&["config", "--quiet"], "config")?;
            let http = Http::new(Duration::from_secs(2))?;
            let address = format!("http://127.0.0.1:{port}");
            let probe = serde_json::json!({"path":"/api/utils/env"});
            for phase in ["initial", "recreated"] {
                execute(&["up", "-d", "--pull", "never"], &format!("{phase}-up"))?;
                let deadline = Instant::now() + Duration::from_secs(30);
                let response = loop {
                    match http.request(&address, &probe) {
                        Ok(response) if response["status"] == 200 => break response,
                        _ => {}
                    }
                    require(
                        Instant::now() < deadline,
                        "Compose backend did not become ready",
                    )?;
                    std::thread::sleep(Duration::from_millis(100));
                };
                http::check(
                    &response,
                    &serde_json::json!({"status":200,"headers":{"x-powered-by":"compose-env-fixture"}}),
                )?;
                let request = if phase == "initial" {
                    serde_json::json!({"path":"/api/settings","method":"PATCH","body":{"composeExample":"retained"}})
                } else {
                    serde_json::json!({"path":"/api/settings"})
                };
                let response = http.request(&address, &request)?;
                http::check(
                    &response,
                    &serde_json::json!({"status":200,"json_contains":{"data":{"composeExample":"retained"}}}),
                )?;
                json::write_new(output.join(format!("{phase}.json")), &response)?;
                execute(&["logs", "--no-color"], &format!("{phase}-logs"))?;
                execute(&["down", "--timeout", "10"], &format!("{phase}-down"))?;
            }
            Ok(())
        })();
        let _ = execute(&["logs", "--no-color"], "final-logs");
        let cleanup = execute(&["down", "--timeout", "10"], "cleanup");
        result?;
        cleanup?;
        Ok(())
    }
    #[test]
    fn owned_probe_cleanup_preserves_other_containers_on_success_timeout_and_launch_failure()
    -> Result {
        let output = process::scratch("docker-cleanup")?;
        let image = std::env::var("TEST_DOCKER_PROBE_IMAGE")?;
        let target = sub_crate_tests::script_target::Target::Docker(image.clone());
        let prefix = format!("nextest-probe-{}", std::process::id());
        let control = format!("{prefix}-control");
        process::checked(
            Command::new("docker").args([
                "create",
                "--name",
                &control,
                "--entrypoint",
                "/bin/sh",
                &image,
                "-c",
                "true",
            ]),
            &output,
            "create-control",
            Duration::from_secs(30),
        )?;
        let result = (|| -> Result {
            for scenario in ["success", "timeout", "launch"] {
                let name = format!("{prefix}-{scenario}");
                let mut command = if scenario == "launch" {
                    process::checked(
                        Command::new("docker").args([
                            "create",
                            "--name",
                            &name,
                            "--entrypoint",
                            "/bin/sh",
                            &image,
                            "-c",
                            "true",
                        ]),
                        &output,
                        "create-launch",
                        Duration::from_secs(30),
                    )?;
                    let mut command = Command::new("missing-nextest-probe-executable");
                    command.args(["--name", &name]);
                    command
                } else {
                    let mut command = Command::new("docker");
                    command.args([
                        "run",
                        "--name",
                        &name,
                        "--entrypoint",
                        "/bin/sh",
                        &image,
                        "-c",
                        if scenario == "timeout" {
                            "printf partial; sleep 120"
                        } else {
                            "printf partial"
                        },
                    ]);
                    command
                };
                let result = target.run(&mut command, &output, scenario);
                match scenario {
                    "launch" => require(result.is_err(), "Launch failure was hidden")?,
                    "timeout" => {
                        let result = result?;
                        require(
                            result.timed_out && !result.success() && result.stdout == "partial",
                            "Probe watchdog or partial output was lost",
                        )?;
                    }
                    _ => {
                        let result = result?;
                        require(
                            result.success() && result.stdout == "partial",
                            "Successful probe output was lost",
                        )?;
                    }
                }
                let removed = process::run(
                    Command::new("docker").args(["container", "inspect", &name]),
                    &output,
                    &format!("removed-{scenario}"),
                    Duration::from_secs(30),
                )?;
                require(!removed.success(), "Owned probe container survived cleanup")?;
                process::checked(
                    Command::new("docker").args(["container", "inspect", &control]),
                    &output,
                    &format!("control-{scenario}"),
                    Duration::from_secs(30),
                )?;
            }
            let mut unowned = Command::new("missing-nextest-probe-executable");
            unowned.args(["--name", "unrelated-container"]);
            require(
                target.run(&mut unowned, &output, "unowned").is_err(),
                "Unowned container cleanup was allowed",
            )?;
            Ok(())
        })();
        process::checked(
            Command::new("docker").args(["rm", "--force", &control]),
            &output,
            "remove-control",
            Duration::from_secs(30),
        )?;
        result
    }
    #[test]
    fn delivery() -> Result {
        sub_crate_tests::distribution::docker(
            &process::scratch("docker-delivery")?,
            &std::env::var("TEST_DOCKER_IMAGE")?,
        )
    }
    #[test]
    fn probe_image_matches_native_runner() -> Result {
        let output = process::scratch("docker-probe-image")?;
        let image = std::env::var("TEST_DOCKER_PROBE_IMAGE")?;
        let inspected = process::checked(
            Command::new("docker").args(["image", "inspect", &image]),
            &output,
            "probe-image-inspect",
            Duration::from_secs(30),
        )?;
        let image = json::parse(&inspected.stdout)?[0].clone();
        let expected = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            _ => return Err("Unsupported native Docker runner".into()),
        };
        require(
            image["Os"] == "linux" && image["Architecture"] == expected,
            "Probe image used another architecture",
        )?;
        json::write_new(output.join("image.json"), &image)
    }
}
