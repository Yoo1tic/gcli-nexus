use std::env;

fn main() {
    dotenvy::dotenv().ok();
    println!("cargo:rerun-if-changed=.env");

    ["SQLX_OFFLINE", "CLAUDE_SYSTEM_PREAMBLE"]
        .into_iter()
        .for_each(|key| {
            println!("cargo:rerun-if-env-changed={}", key);
            if let Ok(val) = env::var(key) {
                println!("cargo:rustc-env={}={}", key, val);
            }
        });
}
