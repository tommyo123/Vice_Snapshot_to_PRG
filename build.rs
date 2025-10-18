fn main() {
    // Platform-specific build configuration
    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        println!("cargo:warning=Building for Windows with dynamic CRT (matches FLTK)");
    }

    #[cfg(all(target_os = "linux", target_env = "musl"))]
    {
        println!("cargo:warning=Building for Linux (musl) with static linking");
        // Musl kan bygge helt statisk
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        println!("cargo:warning=Building for Linux (glibc)");
    }

    #[cfg(target_os = "macos")]
    {
        println!("cargo:warning=Building for macOS");
    }

    // Windows-specific: Add icon resource
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("res/VICE-SNAPSHOT-TO-PRG-CONVERTER.ICO");
        res.compile().unwrap();
    }
}
