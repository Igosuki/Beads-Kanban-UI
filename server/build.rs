use std::fs;
use std::path::Path;

fn main() {
    let out_dir = Path::new("../out");
    if !out_dir.exists() {
        fs::create_dir_all(out_dir).expect("Failed to create out/ directory");
        fs::write(
            out_dir.join("index.html"),
            "<html><body><h1>Frontend not built</h1><p>Run <code>npm run build</code> in the project root first.</p></body></html>",
        )
        .expect("Failed to write placeholder index.html");
    }
    println!("cargo:rerun-if-changed=../out");
}
