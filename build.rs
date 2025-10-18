fn main() {
    // Platform-specific build configuration
    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        println!("cargo:warning=Building for Windows with dynamic CRT (matches FLTK)");

        // Windows-specific: Add icon resource
        let mut res = winres::WindowsResource::new();
        res.set_icon("res/VICE-SNAPSHOT-TO-PRG-CONVERTER.ICO");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile Windows resources: {}", e);
        }
    }

    #[cfg(all(target_os = "linux", target_env = "musl"))]
    {
        println!("cargo:warning=Building for Linux (musl) with static linking");
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        println!("cargo:warning=Building for Linux (glibc)");
    }

    #[cfg(target_os = "macos")]
    {
        println!("cargo:warning=Building for macOS");
    }
}
