use crate::Scope;
use crate::spawner::*;

impl<'a, T, Sp: Spawner<T> + Blocker + Default> Scope<'a, T, Sp> {
    /// A function that creates a scope and immediately awaits,
    /// _blocking the current thread_ for spawned futures to
    /// complete. The outputs of the futures are collected as a
    /// `Vec` and returned along with the output of the block.
    ///
    /// # Safety
    ///
    /// This function is safe to the best of our understanding
    /// as it blocks the current thread until the stream is
    /// driven to completion, implying that all the spawned
    /// futures have completed too. However, care must be taken
    /// to ensure a recursive usage of this function doesn't
    /// lead to deadlocks.
    ///
    /// When scope is used recursively, you may also use the
    /// unsafe `scope_and_*` functions as long as this function
    /// is used at the top level. In this case, either the
    /// recursively spawned should have the same lifetime as the
    /// top-level scope, or there should not be any spurious
    /// future cancellations within the top level scope.
    pub fn scope_and_block<R, F>(f: F) -> (R, Vec<Sp::FutureOutput>)
    where
        T: Send + 'static,
        Sp: Spawner<T> + Blocker,
        F: AsyncFnOnce(&mut Scope<'a, T, Sp>) -> R,
    {
        let mut scope = unsafe { Scope::create(Default::default()) };
        let spawner = Sp::default();
        let block_output = spawner.block_on(f(&mut scope));
        let proc_outputs = spawner.block_on(scope.collect());
        (block_output, proc_outputs)
    }

    /// An asynchronous function that creates a scope and
    /// immediately awaits the stream. The outputs of the
    /// futures are collected as a `Vec` and returned along with
    /// the output of the block.
    ///
    /// # Safety
    ///
    /// This function is _not completely safe_: please see
    /// `cancellation_soundness` in [tests.rs][tests-src] for a
    /// test-case that suggests how this can lead to invalid
    /// memory access if not dealt with care.
    ///
    /// The caller must ensure that the lifetime 'a is valid
    /// until the returned future is fully driven. Dropping the
    /// future is okay, but blocks the current thread until all
    /// spawned futures complete.
    ///
    /// [tests-src]: https://github.com/rmanoka/async-scoped/blob/master/src/tests.rs
    pub async unsafe fn scope_and_collect<R, F>(f: F) -> (R, Vec<Sp::FutureOutput>)
    where
        T: Send + 'static,
        F: AsyncFnOnce(&mut Scope<'a, T, Sp>) -> R,
    {
        let mut scope = unsafe { Scope::create(Default::default()) };
        let block_output = f(&mut scope).await;
        let proc_outputs = scope.collect().await;
        (block_output, proc_outputs)
    }
}
