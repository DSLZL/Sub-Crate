use sub_store_tests::{Result,json,process,require};
use std::{process::Command,time::Duration};
mod docker {
    use super::*;
    #[test]
    fn delivery() -> Result {
        sub_store_tests::distribution::docker(&process::scratch("docker-delivery")?,&std::env::var("TEST_DOCKER_IMAGE")?)
    }
    #[test]
    fn probe_image_matches_native_runner() -> Result {
        let output=process::scratch("docker-probe-image")?;
        let image=std::env::var("TEST_DOCKER_PROBE_IMAGE")?;
        let inspected=process::checked(Command::new("docker").args(["image","inspect",&image]),&output,"probe-image-inspect",Duration::from_secs(30))?;
        let image=json::parse(&inspected.stdout)?[0].clone();let expected=match std::env::consts::ARCH{"x86_64"=>"amd64","aarch64"=>"arm64",_=>return Err("Unsupported native Docker runner".into())};
        require(image["Os"]=="linux"&&image["Architecture"]==expected,"Probe image used another architecture")?;
        json::write_new(output.join("image.json"),&image)
    }
}
