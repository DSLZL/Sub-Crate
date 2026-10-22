use sub_store_tests::{Result,performance::subscription};

#[test]
fn subscription_matrix()->Result{subscription::matrix()}
#[test]
fn subscription_prepare()->Result{subscription::run(true,std::path::Path::new(env!("CARGO_BIN_EXE_performance-fixture")))}
#[test]
fn subscription_pair()->Result{subscription::run(false,std::path::Path::new(env!("CARGO_BIN_EXE_performance-fixture")))}
#[test]
fn subscription_aggregate()->Result{subscription::aggregate()}
#[test]
fn subscription_inspect()->Result{subscription::inspect()}

