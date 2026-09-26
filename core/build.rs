fn main() {
    // SQLite3 Multiple Ciphers 2.5.1 (SQLite 3.53.4). SQLCipher 4 is one of
    // the built-in schemes, so Device Library Plus can be updated without
    // compiling OpenSSL.
    cc::Build::new()
        .file("sqlite3mc/sqlite3mc_amalgamation.c")
        .include("sqlite3mc")
        .define("SQLITE_THREADSAFE", "1")
        .define("SQLITE_TEMP_STORE", "2")
        .define("SQLITE_DQS", "0")
        .define("SQLITE_DEFAULT_MEMSTATUS", "0")
        .define("SQLITE_OMIT_LOAD_EXTENSION", "1")
        .warnings(false)
        .compile("sqlite3mc");
    println!("cargo:rerun-if-changed=sqlite3mc/sqlite3mc_amalgamation.c");
}
