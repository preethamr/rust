use futures_util::FutureExt;
use nomad_core::db::DB;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::path::Path;
use std::{env, future::Future, panic};

use rocksdb::Options;

/// Sets up a db
pub fn setup_db(db_path: String) -> DB {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    rocksdb::DB::open(&opts, db_path)
        .expect("Failed to open db path")
        .into()
}

/// Runs test for a db
pub async fn run_test_db<T, Fut>(test: T)
where
    T: FnOnce(DB) -> Fut + panic::UnwindSafe,
    Fut: Future<Output = ()>,
{
    // RocksDB only allows one unique db handle to be open at a time. Because
    // `cargo test` is multithreaded by default, we use random db pathnames to
    // avoid collisions between 2+ threads
    let rand_path: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let result = {
        let db = setup_db(rand_path.clone());

        let func = panic::AssertUnwindSafe(async { test(db).await });
        func.catch_unwind().await
    };
    let _ = rocksdb::DB::destroy(&Options::default(), rand_path);
    assert!(result.is_ok())
}

pub async fn run_test_with_env<T, Fut>(path: impl AsRef<Path>, test: T)
where
    T: FnOnce() -> Fut + panic::UnwindSafe,
    Fut: Future<Output = ()>,
{
    let result = {
        dotenv::from_filename(path).unwrap();
        let func = panic::AssertUnwindSafe(async { test().await });
        func.catch_unwind().await
    };

    clear_env_vars();
    assert!(result.is_ok())
}

pub fn run_test_with_env_sync<T>(path: impl AsRef<Path>, test: T)
where
    T: FnOnce() + panic::UnwindSafe,
{
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        dotenv::from_filename(path).unwrap();
        test()
    }));

    clear_env_vars();
    assert!(result.is_ok())
}

pub fn clear_env_vars() {
    let env_vars = env::vars();
    for (key, _) in env_vars.into_iter() {
        env::remove_var(key);
    }
}
