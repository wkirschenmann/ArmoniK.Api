//! Compiles what calls into h2-batch's patch when cargo builds against the patched h2: the
//! recipe in `packages/rust/patches/h2-batch` says so with `ARMONIK_H2_BATCH`.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(h2_batch)");
    println!("cargo::rerun-if-env-changed=ARMONIK_H2_BATCH");
    if std::env::var("ARMONIK_H2_BATCH").is_ok_and(|value| value == "1") {
        println!("cargo::rustc-cfg=h2_batch");
    }
}
