use std::{process::Command, time::Duration};
use sub_store_tests::{Result, json, process, require};
mod docker {
    use super::*;
    #[test]
    fn owned_probe_cleanup_preserves_other_containers_on_success_timeout_and_launch_failure() -> Result {
        let output=process::scratch("docker-cleanup")?;let image=std::env::var("TEST_DOCKER_PROBE_IMAGE")?;let target=sub_store_tests::script_target::Target::Docker(image.clone());let prefix=format!("nextest-probe-{}",std::process::id());let control=format!("{prefix}-control");
        process::checked(Command::new("docker").args(["create","--name",&control,"--entrypoint","/bin/sh",&image,"-c","true"]),&output,"create-control",Duration::from_secs(30))?;
        let result=(||->Result {for scenario in ["success","timeout","launch"]{let name=format!("{prefix}-{scenario}");let mut command=if scenario=="launch"{process::checked(Command::new("docker").args(["create","--name",&name,"--entrypoint","/bin/sh",&image,"-c","true"]),&output,"create-launch",Duration::from_secs(30))?;let mut command=Command::new("missing-nextest-probe-executable");command.args(["--name",&name]);command}else{let mut command=Command::new("docker");command.args(["run","--name",&name,"--entrypoint","/bin/sh",&image,"-c",if scenario=="timeout"{"printf partial; sleep 120"}else{"printf partial"}]);command};let result=target.run(&mut command,&output,scenario);
            match scenario{"launch"=>require(result.is_err(),"Launch failure was hidden")?,"timeout"=>{let result=result?;require(result.timed_out&&!result.success()&&result.stdout=="partial","Probe watchdog or partial output was lost")?;},_=>{let result=result?;require(result.success()&&result.stdout=="partial","Successful probe output was lost")?;}}
            let removed=process::run(Command::new("docker").args(["container","inspect",&name]),&output,&format!("removed-{scenario}"),Duration::from_secs(30))?;require(!removed.success(),"Owned probe container survived cleanup")?;process::checked(Command::new("docker").args(["container","inspect",&control]),&output,&format!("control-{scenario}"),Duration::from_secs(30))?;
        }let mut unowned=Command::new("missing-nextest-probe-executable");unowned.args(["--name","unrelated-container"]);require(target.run(&mut unowned,&output,"unowned").is_err(),"Unowned container cleanup was allowed")?;Ok(())})();
        process::checked(Command::new("docker").args(["rm","--force",&control]),&output,"remove-control",Duration::from_secs(30))?;result
    }
    #[test]
    fn delivery() -> Result {
        sub_store_tests::distribution::docker(
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
