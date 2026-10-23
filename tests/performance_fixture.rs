fn main() {
    let result = (|| -> sub_store_tests::Result {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        sub_store_tests::require(
            arguments.len() == 3,
            "Expected mirror directory, fixture port and detector port",
        )?;
        sub_store_tests::performance::detector::run(
            arguments[0].clone().into(),
            arguments[1].parse()?,
            arguments[2].parse()?,
        )
    })();
    if result.is_err() {
        eprintln!("Controlled performance fixture failed");
        std::process::exit(1);
    }
}
