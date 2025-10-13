fn main() {
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("res/VICE-SNAPSHOT-TO-PRG-CONVERTER.ICO");
        res.compile().unwrap();
    }
}