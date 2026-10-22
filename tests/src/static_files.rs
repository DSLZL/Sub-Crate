use crate::Result;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub fn prepare(frontend: &Path) -> Result {
    let files = [
        (
            "index.html",
            b"<main>unchanged frontend entry</main>".as_slice(),
        ),
        (
            "chunks/main-84da2e86.js",
            b"console.log(\"original chunk\")".as_slice(),
        ),
        (
            "chunks/multi-deadbeef.locale-extra.js",
            b"hashed suffix".as_slice(),
        ),
        (
            "chunks/multi-deadbeef.-extra.js",
            b"hashed dash suffix".as_slice(),
        ),
        ("css/main.css", b"body{}".as_slice()),
        ("sw.js", b"worker".as_slice()),
        ("empty.txt", b"".as_slice()),
        (
            "release-deadbeef.dir/index.html",
            b"nested entry".as_slice(),
        ),
        ("text 中文.txt", "Unicode中文🙂\r\n".as_bytes()),
        (".env", b"must never expose dotfiles".as_slice()),
    ];
    let write = |name: &str, bytes: &[u8]| -> Result {
        let path = frontend.join(name);
        std::fs::create_dir_all(path.parent().ok_or("Missing fixture parent")?)?;
        std::fs::write(&path, bytes)?;
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1609459200);
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?
            .set_times(
                std::fs::FileTimes::new()
                    .set_accessed(time)
                    .set_modified(time),
            )?;
        Ok(())
    };
    let gzip = |bytes: &[u8]| -> Result<Vec<u8>> {
        let mut encoder = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes)?;
        Ok(encoder.finish()?)
    };
    for (name, bytes) in files {
        write(name, bytes)?;
        if ["index.html", "chunks/main-84da2e86.js", "css/main.css"].contains(&name) {
            write(&format!("{name}.gz"), &gzip(bytes)?)?;
        }
    }
    write(
        "chunks/only-deadbeef.mjs.gz",
        &gzip(b"compressed-only module")?,
    )?;
    std::fs::write(
        frontend
            .parent()
            .ok_or("Missing fixture state")?
            .join("secret.txt"),
        b"must never expose parent files",
    )?;
    Ok(())
}
