fn main() {
    #[cfg(target_os = "macos")]
    {
        let deps_lib = concat!(env!("CARGO_MANIFEST_DIR"), "/../deps/out/lib");
        println!("cargo:rustc-link-lib=dylib=krun");
        println!("cargo:rustc-link-lib=dylib=slirp");
        println!("cargo:rustc-link-lib=dylib=glib-2.0");
        println!("cargo:rustc-link-lib=dylib=resolv");
        println!("cargo:rustc-link-search={deps_lib}");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/lib");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{deps_lib}");
        println!("cargo:rustc-link-lib=framework=IOSurface");

        verify_libkrun_rpaths(deps_lib);
    }
}

#[cfg(target_os = "macos")]
fn verify_libkrun_rpaths(deps_lib: &str) {
    let libkrun = format!("{deps_lib}/libkrun.dylib");
    let output = std::process::Command::new("otool")
        .args(["-L", &libkrun])
        .output();
    let output = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return,
    };
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("/opt/homebrew/") && trimmed.contains(".dylib") {
            let lib = trimmed.split_whitespace().next().unwrap_or(trimmed);
            panic!(
                "\n\nlibkrun.dylib links to homebrew: {lib}\n\
                 GPU will fail — homebrew virglrenderer has no EGL/ANGLE support.\n\
                 Fix: run install_name_tool to rewrite to @rpath, or re-run deps/build-deps.sh\n\n"
            );
        }
    }
}
