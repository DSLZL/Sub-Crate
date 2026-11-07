# Sourced by rust-single-node.yml: application checkouts before the workspace layout keep `rust/`.
manifest() {
  if [[ -f ".performance/$1/rust/Cargo.toml" ]]; then echo ".performance/$1/rust/Cargo.toml"; else echo ".performance/$1/crates/sub-crate/Cargo.toml"; fi
}
target_dir() {
  if [[ -f ".performance/$1/rust/Cargo.toml" ]]; then echo "$GITHUB_WORKSPACE/.performance/$1/rust/target"; else echo "$GITHUB_WORKSPACE/.performance/$1/target"; fi
}
