//! Regenerates `assets/config.example.toml` from the real defaults.
//! Run with: cargo run -p orchestra-core --example dump_config
fn main() {
    print!("{}", orchestra_core::config::Config::default().to_toml());
}
