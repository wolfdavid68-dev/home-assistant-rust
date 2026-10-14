//! Les migrations SQL sont intégrées au programme à la compilation
//! (`sqlx::migrate!`). Cargo ne sait pas, tout seul, qu'il faut recompiler
//! quand on **ajoute** un fichier dans `migrations/` : on le lui dit ici.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
