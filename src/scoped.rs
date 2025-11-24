use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Remote, RemoteHandle};
use futures::stream::FuturesOrdered;
use futures::{Future, FutureExt, Stream, StreamExt};

use pin_project::*;

use crate::spawner::*;

#[pin_project]
pub struct ScopedSpawnHandle<H> {
    channel: Option<oneshot::Sender<()>>,
    #[pin]
    handle: H,
}

impl<T, H: Future<Output = T>> Future for ScopedSpawnHandle<H> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let p = self.project();
        let result = ready!(p.handle.poll(cx));
        let _ = p.channel.take().unwrap().send(());
        Poll::Ready(result)
    }
}

/// A scope to allow controlled spawning of non 'static
/// futures. Futures can be spawned using `spawn` or
/// `spawn_cancellable` methods.
///
/// # Safety
///
/// This type uses `Drop` implementation to guarantee
/// safety. It is not safe to forget this object unless it
/// is driven to completion.
#[pin_project(PinnedDrop)]
pub struct Scope<'a, T, Sp: Spawner<T> + Blocker> {
    spawner: Option<Sp>,
    len: usize,
    #[pin]
    futs: FuturesOrdered<Remote<Sp::SpawnHandle>>,
    abort_handles: Vec<AbortHandle>,

    // Future proof against variance changes
    _marker: PhantomData<fn(&'a ()) -> &'a ()>,
    _spawn_marker: PhantomData<Sp>,
}

impl<'a, T: Send + 'static, Sp: Spawner<T> + Blocker> Scope<'a, T, Sp> {
    /// Create a Scope object.
    ///
    /// # Safety
    /// This function is unsafe as `futs` may hold futures
    /// which have to be manually driven to completion.
    pub unsafe fn create(spawner: Sp) -> Self {
        Scope {
            spawner: Some(spawner),
            len: 0,
            futs: FuturesOrdered::new(),
            abort_handles: vec![],
            _marker: PhantomData,
            _spawn_marker: PhantomData,
        }
    }

    fn spawner(&self) -> &Sp {
        self.spawner
            .as_ref()
            .expect("invariant:spawner is always available until scope is dropped")
    }

    /// Spawn a future with the executor's `task::spawn` functionality. The
    /// future is expected to be driven to completion before 'a expires.
    pub fn spawn<F: Future<Output = T> + Send + 'a>(
        &mut self,
        f: F,
    ) -> ScopedSpawnHandle<Sp::FutureOutput> {
        let handle = self.spawner().spawn(unsafe {
            std::mem::transmute::<
                std::pin::Pin<std::boxed::Box<dyn futures::Future<Output = T>>>,
                std::pin::Pin<std::boxed::Box<dyn futures::Future<Output = T> + std::marker::Send>>,
            >(Box::pin(f) as Pin<Box<dyn Future<Output = T>>>)
        });
        let (handle, remote_handle) = handle.remote_handle();
        self.futs.push_back(handle);
        self.len += 1;
        ScopedSpawnHandle(remote_handle)
    }

    /// Spawn a cancellable future with the executor's `task::spawn`
    /// functionality.
    ///
    /// The future is cancelled if the `Scope` is dropped
    /// pre-maturely. It can also be cancelled by explicitly
    /// calling (and awaiting) the `cancel` method.
    #[inline]
    pub fn spawn_cancellable<F: Future<Output = T> + Send + 'a, Fu: FnOnce() -> T + Send + 'a>(
        &mut self,
        f: F,
        default: Fu,
    ) -> ScopedSpawnHandle<Sp::FutureOutput> {
        let (h, reg) = AbortHandle::new_pair();
        self.abort_handles.push(h);
        let fut = Abortable::new(f, reg);
        self.spawn(async { fut.await.unwrap_or_else(|_| default()) })
    }

    /// Spawn a function as a blocking future with executor's `spawn_blocking`
    /// functionality.
    ///
    /// The future is cancelled if the `Scope` is dropped
    /// pre-maturely. It can also be cancelled by explicitly
    /// calling (and awaiting) the `cancel` method.
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'a>(
        &mut self,
        f: F,
    ) -> ScopedSpawnHandle<<Sp as Spawner<T>>::FutureOutput>
    where
        Sp: FuncSpawner<T, SpawnHandle = <Sp as Spawner<T>>::SpawnHandle>,
    {
        let handle = self.spawner().spawn_func(unsafe {
            std::mem::transmute::<
                std::boxed::Box<dyn std::ops::FnOnce() -> T + std::marker::Send>,
                std::boxed::Box<dyn std::ops::FnOnce() -> T + std::marker::Send>,
            >(Box::new(f) as Box<dyn FnOnce() -> T + Send>)
        });
        let (handle, remote_handle) = handle.remote_handle();
        self.futs.push_back(handle);
        self.len += 1;
        ScopedSpawnHandle(remote_handle)
    }
}

impl<'a, T, Sp: Spawner<T> + Blocker> Scope<'a, T, Sp> {
    /// Cancel all futures spawned with cancellation.
    #[inline]
    pub fn cancel(&mut self) {
        for h in self.abort_handles.drain(..) {
            h.abort();
        }
    }

    /// Total number of futures spawned in this scope.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of futures remaining in this scope.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.futs.len()
    }
}

impl<'a, T, Sp: Spawner<T> + Blocker> Stream for Scope<'a, T, Sp> {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.project().futs.poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining(), Some(self.remaining()))
    }
}

#[pinned_drop]
impl<'a, T, Sp: Spawner<T> + Blocker> PinnedDrop for Scope<'a, T, Sp> {
    fn drop(mut self: Pin<&mut Self>) {
        if self.remaining() > 0 {
            let spawner = self
                .spawner
                .take()
                .expect("invariant:spawner must be taken only on drop");
            spawner.block_on(async {
                self.cancel();
                self.project().futs.count().await;
            });
        }
    }
}
