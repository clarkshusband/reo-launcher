// `sqlx::migrate!()` embeds the migrations at compile time so a new or edited
// file has to trigger a rebuild of this crate
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
