use async_scoped::{
    Scope,
    spawner::{Blocker, Spawner},
};
use futures::future::pending;
use std::hint::black_box;

async fn spawn_and_collect<'a, Sp: Spawner<usize> + Blocker>(s: &mut Scope<'a, usize, Sp>) {
    const INPUT_SIZE: usize = 1000000;
    const MAX_DELAY: usize = 1 << 8;
    for i in 0..INPUT_SIZE {
        let delay = i & MAX_DELAY;
        s.spawn(async move {
            let _ = async_std::future::timeout(
                std::time::Duration::from_millis((MAX_DELAY - delay) as u64),
                pending::<()>(),
            )
            .await;
            i
        });
    }
}

fn main() {
    #[cfg(all(not(feature = "use-tokio"), feature = "use-async-std"))]
    {
        // Async-std runtime does not have a straightforward way to configure multi-threaded
        // runtime: https://docs.rs/async-std/latest/async_std/index.html#runtime-configuration
        // ASYNC_STD_THREAD_COUNT environment variable must be used to match the Tokio benchmark
        use async_scoped::AsyncStdScope;
        AsyncStdScope::scope_and_block_forever(black_box(spawn_and_collect));
    }
    #[cfg(feature = "use-tokio")]
    {
        use async_scoped::TokioScope;
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .build()
            .unwrap()
            .block_on(
                async move { TokioScope::scope_and_block_forever(black_box(spawn_and_collect)) },
            )
    }
}
