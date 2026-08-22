//! Declarative async state for GPUI, in the shape Solid, React Query, and
//! Svelte settled on: a resource keyed by its input.
//!
//! Declare an async source and the key it depends on. When the key changes the
//! in-flight work is abandoned and its result discarded. The view reads a
//! two-state value. No channel, no polling pump, no generation counter.

use gpui::{App, AppContext, ElementId, Entity, Task, Window};
use std::future::Future;
use std::panic::Location;

/// What the view matches on.
///
/// There is deliberately no `Failed` variant. Nothing that uses this today can
/// fail, and a fallible source can return `Result<T, E>` as its `T`. Add the
/// variant when a caller needs it, not before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource<T> {
    Loading,
    Ready(T),
}

impl<T> Resource<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Resource::Loading => None,
            Resource::Ready(value) => Some(value),
        }
    }
}

/// The entity `use_resource` hands back. Read it with `state.read(cx).value()`.
///
/// The key lives beside the value so a finishing task can prove it is still
/// the current one before writing. That check is the whole point of the hook:
/// it is the `build_id` comparison every GPUI app hand-rolls, written once.
pub struct ResourceState<K, T> {
    /// `None` until the first render starts work, which is what makes the
    /// start-work check a single branch instead of two.
    key: Option<K>,
    value: Resource<T>,
    /// Held so that replacing it on a key change drops the superseded task.
    task: Option<Task<()>>,
}

impl<K, T> ResourceState<K, T> {
    pub fn value(&self) -> &Resource<T> {
        &self.value
    }
}

/// Async state keyed to `key`, identified by the caller's source location.
///
/// Call this only during `Render::render`, `RenderOnce::render`, or an
/// `Element` drawing function. Two logical instances created at one source
/// line share state — use [`use_keyed_resource`] for lists.
#[track_caller]
pub fn use_resource<K, T, Fut>(
    window: &mut Window,
    cx: &mut App,
    key: K,
    source: impl FnOnce(K) -> Fut,
) -> Entity<ResourceState<K, T>>
where
    K: PartialEq + Clone + Send + 'static,
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    use_keyed_resource(
        ElementId::CodeLocation(*Location::caller()),
        window,
        cx,
        key,
        source,
    )
}

/// [`use_resource`] with an explicit id, for when one source location renders
/// many instances — React's `key` prop.
///
/// The source future runs on the background executor, so it must be `Send`.
/// That rules out capturing GPUI handles; capture plain data instead.
pub fn use_keyed_resource<K, T, Fut>(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    key: K,
    source: impl FnOnce(K) -> Fut,
) -> Entity<ResourceState<K, T>>
where
    K: PartialEq + Clone + Send + 'static,
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let state = window.use_keyed_state(id, cx, |_, _| ResourceState {
        key: None,
        value: Resource::Loading,
        task: None,
    });

    if state.read(cx).key.as_ref() == Some(&key) {
        return state;
    }

    state.update(cx, |this, cx| {
        this.key = Some(key.clone());
        this.value = Resource::Loading;

        let future = source(key.clone());
        // Storing the task means this line drops the previous one, and a
        // dropped GPUI task is cancelled. That is the primary protection: a
        // superseded task stops at its next await point and never delivers.
        // Measured in tests/mechanism.rs — five keys start, one finishes.
        //
        // Work with no await point still runs to completion on its background
        // thread; only delivery is prevented. Saving that CPU needs
        // cancellation checks inside the source itself, which is stage 3.
        this.task = Some(cx.spawn(async move |this, cx| {
            let value = cx.background_spawn(future).await;
            this.update(cx, |this, cx| {
                // Backstop, in case a future ever outlives its cancellation:
                // a key change mid-flight obsoletes this result.
                if this.key.as_ref() == Some(&key) {
                    this.value = Resource::Ready(value);
                    cx.notify();
                }
            })
            .ok();
        }));
    });

    state
}

