// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where an engine's calls run: straight in on the caller's thread, or on
//! the one thread a pinned engine lives on.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::any::Any;
use core::cell::{Cell, RefCell};
use core::marker::PhantomData;
use core::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::sync::mpsc;
use std::thread::{self, ThreadId};

use crate::engine::{Engine, ThreadModel};
use crate::error::BridgeError;

/// A call queued to an engine thread.
struct Job {
    /// Whether it may run while another call waits in [`yield_calls`]: a
    /// call into a library may, opening or closing one may not.
    nestable: bool,
    run: Run,
}

/// What a queued call does with the engine its thread owns.
type Run = Box<dyn FnOnce(&dyn Any) + Send>;

/// What a pinned engine's thread serves, kept where a call running on it
/// reaches it.
struct Serving {
    jobs: mpsc::Receiver<Job>,
    /// Jobs that arrived while a call waited and may not run inside it, in
    /// the order they came: the thread runs them once no call is open.
    held: RefCell<VecDeque<Job>>,
    /// How many calls are waiting in [`yield_calls`] on this thread.
    waiting: Cell<usize>,
}

std::thread_local! {
    /// The engine a thread the bridge started owns, so a call made on that
    /// thread reaches it without queueing behind itself.
    static OWNED: RefCell<Option<Rc<dyn Any>>> = const { RefCell::new(None) };
    static SERVING: RefCell<Option<Rc<Serving>>> = const { RefCell::new(None) };
}

/// The most calls that may wait inside one another on a thread: each
/// takes stack, and a wait that cannot nest further runs nothing.
const MOST_WAITING: usize = 8;
/// The most calls one [`yield_calls`] runs, so a flood of calls cannot
/// keep the one that yielded from going on.
const MOST_PER_YIELD: usize = 64;

/// Runs the calls queued behind the one running on an engine's thread now,
/// each to its end, and answers how many ran. A script that waits for
/// something for a long time (a program to end) calls this between looks,
/// so the engine's one thread answers other calls meanwhile: a `cancel` of
/// what it waits for among them. A call that runs here runs inside the
/// waiting one, so it returns before the waiting one goes on, and what it
/// changes the waiting one finds changed.
///
/// Only calls into libraries run here (a slot, creating or dropping an
/// instance, asking whether a provider is available); opening, closing and
/// refreshing libraries wait until no call is open. It answers 0, running
/// nothing, anywhere but on the thread of a [pinned](ThreadModel::Pinned)
/// engine, and when eight calls already wait inside one another.
pub fn yield_calls() -> usize {
    let Some(serving) = SERVING.with(|s| s.borrow().clone()) else {
        return 0;
    };
    let Some(engine) = OWNED.with(|owned| owned.borrow().clone()) else {
        return 0;
    };
    if serving.waiting.get() >= MOST_WAITING {
        return 0;
    }
    serving.waiting.set(serving.waiting.get() + 1);
    let _leave = Waiting(&serving.waiting);
    let mut ran = 0;
    while ran < MOST_PER_YIELD {
        match serving.jobs.try_recv() {
            Ok(job) if job.nestable => {
                (job.run)(&*engine);
                ran += 1;
            }
            Ok(job) => serving.held.borrow_mut().push_back(job),
            Err(_) => break,
        }
    }
    ran
}

struct Waiting<'a>(&'a Cell<usize>);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// How one source reaches its engine.
pub(crate) struct Runner<E> {
    way: Way,
    threads: ThreadModel,
    _engine: PhantomData<fn() -> E>,
}

impl<E> Clone for Runner<E> {
    fn clone(&self) -> Runner<E> {
        Runner {
            way: self.way.clone(),
            threads: self.threads,
            _engine: PhantomData,
        }
    }
}

#[derive(Clone)]
enum Way {
    /// The engine itself, called on the caller's thread.
    Shared(Arc<dyn Any + Send + Sync>),
    /// The engine's own thread, and the queue to it.
    Pinned {
        queue: mpsc::Sender<Job>,
        thread: ThreadId,
    },
}

fn panicked(payload: Box<dyn Any + Send>) -> BridgeError {
    // Dropping a payload can panic again.
    core::mem::forget(payload);
    BridgeError::EngineGone
}

impl<E: Engine> Runner<E> {
    /// An engine shared by every caller: `Any` or `Locked`.
    pub(crate) fn shared(engine: E) -> Runner<E>
    where
        E: Send + Sync,
    {
        Runner {
            way: Way::Shared(Arc::new(engine)),
            threads: E::THREADS,
            _engine: PhantomData,
        }
    }

    /// Starts the engine on a thread of its own with `make`, its stack
    /// `stack` bytes when given, and waits until it has started or refused.
    pub(crate) fn pinned(
        name: &str,
        stack: Option<usize>,
        make: impl FnOnce() -> Result<E, BridgeError> + Send + 'static,
    ) -> Result<Runner<E>, BridgeError> {
        let (queue, jobs) = mpsc::channel::<Job>();
        let (ready, started) = mpsc::channel::<Result<(), BridgeError>>();
        let mut builder = thread::Builder::new().name(name.to_string());
        if let Some(bytes) = stack {
            builder = builder.stack_size(bytes);
        }
        let spawned = builder
            .spawn(move || {
                let engine = match catch_unwind(AssertUnwindSafe(make)) {
                    Ok(Ok(engine)) => engine,
                    Ok(Err(e)) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                    Err(payload) => {
                        let _ = ready.send(Err(panicked(payload)));
                        return;
                    }
                };
                let engine: Rc<dyn Any> = Rc::new(engine);
                OWNED.with(|owned| *owned.borrow_mut() = Some(Rc::clone(&engine)));
                let serving = Rc::new(Serving {
                    jobs,
                    held: RefCell::new(VecDeque::new()),
                    waiting: Cell::new(0),
                });
                SERVING.with(|s| *s.borrow_mut() = Some(Rc::clone(&serving)));
                let _ = ready.send(Ok(()));
                // Until every handle on the queue is gone.
                loop {
                    let held = serving.held.borrow_mut().pop_front();
                    let Some(job) = held.or_else(|| serving.jobs.recv().ok()) else {
                        break;
                    };
                    (job.run)(&*engine);
                }
                SERVING.with(|s| s.borrow_mut().take());
                OWNED.with(|owned| owned.borrow_mut().take());
            })
            .map_err(|e| {
                BridgeError::Unavailable(match stack {
                    Some(bytes) => format!(
                        "the engine thread did not start with a stack of {bytes} bytes: {e}"
                    ),
                    None => format!("the engine thread did not start: {e}"),
                })
            })?;
        started.recv().map_err(|_| BridgeError::EngineGone)??;
        Ok(Runner {
            way: Way::Pinned {
                queue,
                thread: spawned.thread().id(),
            },
            threads: ThreadModel::Pinned,
            _engine: PhantomData,
        })
    }

    /// Where calls run.
    pub(crate) fn threads(&self) -> ThreadModel {
        self.threads
    }

    /// Runs `f` with the engine where its thread model says, and answers
    /// what it answered. A panic in `f` is caught and answers
    /// `EngineGone`; so does a queue whose thread has ended.
    pub(crate) fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&E) -> T + Send + 'static,
    ) -> Result<T, BridgeError> {
        self.queued(false, f)
    }

    /// As [`run`](Runner::run), for a call into a library: one that may
    /// run inside another that waits in [`yield_calls`].
    pub(crate) fn run_call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&E) -> T + Send + 'static,
    ) -> Result<T, BridgeError> {
        self.queued(true, f)
    }

    fn queued<T: Send + 'static>(
        &self,
        nestable: bool,
        f: impl FnOnce(&E) -> T + Send + 'static,
    ) -> Result<T, BridgeError> {
        match &self.way {
            Way::Shared(engine) => {
                let engine = engine.downcast_ref::<E>().ok_or(BridgeError::EngineGone)?;
                catch_unwind(AssertUnwindSafe(|| f(engine))).map_err(panicked)
            }
            Way::Pinned { queue, thread } => {
                if thread::current().id() == *thread {
                    // The engine calling itself through the host: its own
                    // thread is busy with the outer call, so run here.
                    let engine = OWNED
                        .with(|owned| owned.borrow().clone())
                        .ok_or(BridgeError::EngineGone)?;
                    let engine = engine.downcast_ref::<E>().ok_or(BridgeError::EngineGone)?;
                    return catch_unwind(AssertUnwindSafe(|| f(engine))).map_err(panicked);
                }
                let (answer, answered) = mpsc::sync_channel(1);
                let job = Job {
                    nestable,
                    run: Box::new(move |engine: &dyn Any| {
                        let result = match engine.downcast_ref::<E>() {
                            Some(engine) => {
                                catch_unwind(AssertUnwindSafe(|| f(engine))).map_err(panicked)
                            }
                            None => Err(BridgeError::EngineGone),
                        };
                        let _ = answer.send(result);
                    }),
                };
                queue.send(job).map_err(|_| BridgeError::EngineGone)?;
                answered.recv().map_err(|_| BridgeError::EngineGone)?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::Opened;
    use crate::manifest::{Manifest, ManifestFormat};
    use core::time::Duration;
    use std::sync::Mutex;

    /// An engine that is not `Send`: only a pinned runner can hold it.
    struct Local {
        on: ThreadId,
        _not_send: Rc<()>,
    }

    impl Engine for Local {
        type Library = u64;
        const THREADS: ThreadModel = ThreadModel::Pinned;
        const MANIFESTS: ManifestFormat = ManifestFormat {
            serves: "local",
            id_key: "id",
            code_extensions: &[],
            skip_dirs: &[],
        };
        fn start(_: &Config) -> Result<Local, BridgeError> {
            Ok(Local {
                on: thread::current().id(),
                _not_send: Rc::new(()),
            })
        }
        fn open(&self, _: &Manifest, _: &str) -> Result<Opened<u64>, BridgeError> {
            Err(BridgeError::engine("nothing to open"))
        }
        fn close(&self, _: u64) {}
    }

    fn pinned() -> Runner<Local> {
        let config = Config::from_map(&guatiao::Map::new()).expect("empty");
        Runner::pinned("bridge-test", None, move || Local::start(&config)).expect("started")
    }

    /// Recurses until `bytes` of stack lie below the first frame.
    fn use_stack(bytes: usize) -> usize {
        #[inline(never)]
        fn down(start: usize, bytes: usize) -> usize {
            let pad = core::hint::black_box([0u8; 4096]);
            let here = core::ptr::addr_of!(pad) as usize;
            if start.saturating_sub(here) >= bytes {
                return usize::from(pad[0]);
            }
            core::hint::black_box(down(start, bytes)) + usize::from(pad[1])
        }
        let top = 0u8;
        down(core::ptr::addr_of!(top) as usize, bytes)
    }

    #[test]
    fn a_pinned_engine_runs_on_the_stack_it_asked_for() {
        // Far past any platform's default thread stack, which would abort.
        let config = Config::from_map(&guatiao::Map::new()).expect("empty");
        let runner: Runner<Local> = Runner::pinned("bridge-stack", Some(256 << 20), move || {
            Local::start(&config)
        })
        .expect("started");
        assert_eq!(runner.run(|_| use_stack(64 << 20)).expect("ran"), 0);
    }

    #[test]
    fn a_pinned_engine_runs_every_call_on_its_own_thread() {
        let runner = pinned();
        let here = thread::current().id();
        let (on, ran) = runner.run(|e| (e.on, thread::current().id())).expect("ran");
        assert_eq!(on, ran, "on the thread it started on");
        assert_ne!(ran, here);
        let from_another = {
            let runner = runner.clone();
            thread::spawn(move || runner.run(|_| thread::current().id()).expect("ran"))
                .join()
                .expect("joined")
        };
        assert_eq!(from_another, ran);
    }

    #[test]
    fn a_call_the_engine_makes_on_its_own_thread_runs_there_and_then() {
        let runner = pinned();
        let inner = runner.clone();
        let nested = runner
            .run(move |_| inner.run(|_| thread::current().id()))
            .expect("the outer call ran")
            .expect("the inner call ran without waiting behind the outer");
        assert_eq!(nested, runner.run(|_| thread::current().id()).expect("ran"));
    }

    #[test]
    fn a_panic_in_a_call_is_refused_and_the_engine_carries_on() {
        let runner = pinned();
        let e = runner.run(|_| -> u8 { panic!("asked to") }).unwrap_err();
        assert_eq!(e, BridgeError::EngineGone);
        assert_eq!(runner.run(|_| 7).expect("still there"), 7);
    }

    /// Calls `f` from a thread of its own and hands back its answer.
    fn from_a_thread<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
    ) -> thread::JoinHandle<T> {
        thread::spawn(f)
    }

    #[test]
    fn a_call_that_waits_lets_a_call_behind_it_run() {
        let runner = pinned();
        let (waiting, is_waiting) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let long = {
            let runner = runner.clone();
            from_a_thread(move || {
                runner
                    .run_call(move |_| {
                        waiting.send(()).expect("told");
                        let mut served = 0;
                        // Until the other call has answered from outside.
                        while released.try_recv().is_err() {
                            served += yield_calls();
                            thread::sleep(Duration::from_millis(5));
                        }
                        served
                    })
                    .expect("ran")
            })
        };
        is_waiting.recv().expect("started");
        // Answered while the long call is still running: it could only run
        // inside it.
        assert_eq!(runner.run_call(|_| 7).expect("answered"), 7);
        release.send(()).expect("released");
        assert_eq!(long.join().expect("joined"), 1);
    }

    #[test]
    fn a_call_that_is_not_into_a_library_waits_for_the_one_that_yields() {
        let runner = pinned();
        let (waiting, is_waiting) = mpsc::channel::<()>();
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let long = {
            let runner = runner.clone();
            let order = Arc::clone(&order);
            from_a_thread(move || {
                runner
                    .run_call(move |_| {
                        waiting.send(()).expect("told");
                        thread::sleep(Duration::from_millis(300));
                        let served = yield_calls();
                        order.lock().expect("lock").push("long ended");
                        served
                    })
                    .expect("ran")
            })
        };
        is_waiting.recv().expect("started");
        {
            let order = Arc::clone(&order);
            runner
                .run(move |_| order.lock().expect("lock").push("opened"))
                .expect("ran");
        }
        assert_eq!(
            long.join().expect("joined"),
            0,
            "nothing nestable was queued"
        );
        assert_eq!(*order.lock().expect("lock"), ["long ended", "opened"]);
    }

    #[test]
    fn yielding_off_the_engine_thread_runs_nothing() {
        let runner = pinned();
        assert_eq!(yield_calls(), 0);
        assert_eq!(runner.run(|_| 3).expect("ran"), 3);
    }

    #[test]
    fn calls_wait_inside_one_another_only_so_deep() {
        let runner = pinned();
        // Each call yields, which runs the next queued one inside it.
        let depth = Arc::new(Mutex::new((0usize, 0usize)));
        let (queued, all_queued) = mpsc::channel::<()>();
        let mut handles = Vec::new();
        for _ in 0..12 {
            let runner = runner.clone();
            let depth = Arc::clone(&depth);
            let queued = queued.clone();
            handles.push(from_a_thread(move || {
                runner
                    .run_call(move |_| {
                        let _ = queued.send(());
                        let mut state = depth.lock().expect("lock");
                        state.0 += 1;
                        state.1 = state.1.max(state.0);
                        drop(state);
                        thread::sleep(Duration::from_millis(150));
                        yield_calls();
                        depth.lock().expect("lock").0 -= 1;
                    })
                    .expect("ran")
            }));
        }
        for _ in 0..12 {
            all_queued
                .recv_timeout(Duration::from_secs(30))
                .expect("queued");
        }
        for handle in handles {
            handle.join().expect("joined");
        }
        let deepest = depth.lock().expect("lock").1;
        assert!((2..=MOST_WAITING + 1).contains(&deepest), "{deepest}");
    }
}
