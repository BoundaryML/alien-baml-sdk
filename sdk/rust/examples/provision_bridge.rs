//! Fetches the published BAML runtime library (`bridge_cffi`) for this
//! platform into `$BAML_HOME/bridges/<version>/<target>/`, verified against
//! Boundary's release manifest by sha256, and prints its path. The scripts run
//! this once, then point every Go and Rust run at the file (`BAML_BRIDGE_PATH`)
//! with downloads disabled.

fn main() {
    match baml_bridge::loader::ensure_library_cached() {
        Ok(path) => println!("{}", path.display()),
        Err(error) => {
            eprintln!("cannot provision the BAML runtime library: {error}");
            std::process::exit(1);
        }
    }
}
