use std::env;

fn main() {
    dotenvy::dotenv().ok();
    println!("cargo:rerun-if-changed=.env");

    ["GCLI_CLIENT_ID", "GCLI_CLIENT_SECRET", "SQLX_OFFLINE"]
        .into_iter()
        .filter_map(|key| env::var(key).ok().map(|val| (key, val)))
        .for_each(|(key, val)| {
            println!("cargo:rustc-env={}={}", key, val);
            println!("cargo:rerun-if-env-changed={}", key);
        });
}
