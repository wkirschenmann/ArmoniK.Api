//! Point 1: which dispatcher receives an event, by the thread that emits it.

use observability_spike::runtime::{Front, ObsRuntime};
use observability_spike::testkit::{collect, Collector};
use tracing::instrument::WithSubscriber;

const TARGET: &str = "armonik_transport::routing";

fn started(front: Front) -> (ObsRuntime, Box<Collector>) {
    let runtime = ObsRuntime::new(front);
    let collector = Box::new(Collector::default());
    runtime
        .obs
        .set_log_callback(collect, collector.ctx(), "")
        .expect("the default filter parses");
    (runtime, collector)
}

fn emit(what: &'static str) {
    tracing::info!(target: "armonik_transport::routing", "{what}");
}

fn routing(front: Front) {
    let (a, in_a) = started(front);
    let (b, in_b) = started(front);
    let channel_a = a.start_channel_thread();
    let channel_b = b.start_channel_thread();

    // A task on a channel's thread, a task it spawns itself, and the blocking pool of that thread.
    let handle = channel_a.handle.clone();
    a.tokio
        .block_on(channel_a.handle.spawn(async move {
            emit("a: channel task");
            tokio::spawn(async { emit("a: nested task") }).await.unwrap();
            handle
                .spawn_blocking(|| emit("a: blocking pool"))
                .await
                .unwrap();
        }))
        .unwrap();
    b.tokio
        .block_on(channel_b.handle.spawn(async { emit("b: channel task") }))
        .unwrap();

    // The runtime's own worker, and a thread of its blocking pool.
    a.tokio
        .block_on(a.tokio.handle().spawn(async { emit("a: worker task") }))
        .unwrap();
    a.tokio
        .block_on(a.tokio.handle().spawn_blocking(|| emit("a: worker blocking")))
        .unwrap();

    // A host thread inside an ak_* call, and one outside.
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _inside = b.scope();
            emit("b: host thread inside a call");
        });
        scope.spawn(|| emit("nobody: host thread outside any call"));
    });

    let mut got_a = in_a.messages();
    let mut got_b = in_b.messages();
    got_a.sort();
    got_b.sort();
    assert_eq!(
        got_a,
        [
            "a: blocking pool",
            "a: channel task",
            "a: nested task",
            "a: worker blocking",
            "a: worker task"
        ],
        "{front:?}"
    );
    assert_eq!(
        got_b,
        ["b: channel task", "b: host thread inside a call"],
        "{front:?}"
    );
    let _ = TARGET;
}

#[test]
fn each_event_reaches_the_runtime_whose_thread_emitted_it_bare() {
    routing(Front::Bare);
}

#[test]
fn each_event_reaches_the_runtime_whose_thread_emitted_it_layered() {
    routing(Front::Layered);
}

/// A task is attributed to the thread that polls it, not to the runtime it was written for.
#[test]
fn a_task_moved_to_another_runtime_logs_there_unless_it_carries_its_dispatcher() {
    let (a, in_a) = started(Front::Bare);
    let (b, in_b) = started(Front::Bare);
    let channel_b = b.start_channel_thread();

    let moved = async { emit("moved task") };
    b.tokio.block_on(channel_b.handle.spawn(moved)).unwrap();
    assert_eq!(in_a.messages(), Vec::<String>::new());
    assert_eq!(in_b.messages(), ["moved task"]);
    in_b.take();

    let carried = async { emit("carried task") }.with_subscriber(a.dispatch.clone());
    b.tokio.block_on(channel_b.handle.spawn(carried)).unwrap();
    assert_eq!(in_a.messages(), ["carried task"]);
    assert_eq!(in_b.messages(), Vec::<String>::new());
}

/// `Runtime::block_on` polls its future on the calling thread: without a scope, that thread's
/// dispatcher decides.
#[test]
fn block_on_from_a_host_thread_needs_a_scope() {
    let (a, in_a) = started(Front::Bare);
    std::thread::scope(|scope| {
        scope.spawn(|| a.tokio.block_on(async { emit("block_on without scope") }));
        scope.spawn(|| {
            let _inside = a.scope();
            a.tokio.block_on(async { emit("block_on with scope") })
        });
    });
    assert_eq!(in_a.messages(), ["block_on with scope"]);
}
