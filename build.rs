fn main() {
    // Only run on Windows
    #[cfg(windows)]
    {
        // Embed the icon resource into the executable
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}
