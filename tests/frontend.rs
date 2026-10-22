use sub_store_tests::{Result, distribution, process, reference, server::Backend, service};

#[test]
fn frontend_delivery() -> Result {
    distribution::frontend(&process::scratch("frontend-delivery")?)?;
    Ok(())
}
mod frontend {
    use super::*;
    #[test]
    fn node() -> Result {
        sub_store_tests::frontend::run(
            &process::scratch("frontend-node")?,
            Backend::Node(reference::node()),
            false,
        )
    }
    #[test]
    fn rust_merged() -> Result {
        sub_store_tests::frontend::run(
            &process::scratch("frontend-rust-merged")?,
            service::target()?,
            false,
        )
    }
    #[test]
    fn rust_standalone() -> Result {
        sub_store_tests::frontend::run(
            &process::scratch("frontend-rust-standalone")?,
            service::target()?,
            true,
        )
    }
}
