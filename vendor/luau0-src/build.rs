// Makes the crate's fingerprint follow its C++ sources: without a build script Cargo fingerprints only the Rust files,
// so a patched VM (luau/) left every existing build linking the old one (mlua-sys compiles these sources in its own
// build script, which reruns only when this crate rebuilds)
fn main() {
	println!("cargo:rerun-if-changed=luau");
}
