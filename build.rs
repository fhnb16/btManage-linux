fn main() {
    // Both are read with option_env! in src/update.rs.
    println!("cargo:rerun-if-env-changed=BTM_VERSION");
    println!("cargo:rerun-if-env-changed=BTM_UPDATE_REPOSITORY");
    slint_build::compile("ui/app.slint").expect("ui/app.slint compiles");
}
