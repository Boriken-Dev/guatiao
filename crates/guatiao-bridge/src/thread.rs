// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where an engine's calls run: straight in on the caller's thread, or on
//! the one thread a pinned engine lives on.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::any::Any;
use core::cell::RefCell;
use core::marker::PhantomData;
use core::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::sync::mpsc;
use std::thread::{self, ThreadId};

use crate::engine::{Engine, ThreadModel};
use crate::error::BridgeError;

/// A call queued to an engine thread.
type Job = Box<dyn FnOnce(&dyn Any) + Send>;

std::thread_local! {
    /// The engine a thread the bridge started owns, so a call made on that
    /// thread reaches it without queueing behind itself.
    static OWNED: RefCell<Option<Rc<dyn Any>>> = const { RefCell::new(None) };
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

    /// Starts the engine on a thread of its own with `make`, and waits until
    /// it has started or refused.
    pub(crate) fn pinned(
        name: &str,
        make: impl FnOnce() -> Result<E, BridgeError> + Send + 'static,
    ) -> Result<Runner<E>, BridgeError> {
        let (queue, jobs) = mpsc::channel::<Job>();
        let (ready, started) = mpsc::channel::<Result<(), BridgeError>>();
        let spawned = thread::Builder::new()
            .name(name.to_string())
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
                let _ = ready.send(Ok(()));
                // Until every handle on the queue is gone.
                for job in jobs {
                    job(&*engine);
                }
                OWNED.with(|owned| owned.borrow_mut().take());
            })
            .map_err(|e| {
                BridgeError::Unavailable(format!("the engine thread did not start: {e}"))
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
                let job: Job = Box::new(move |engine: &dyn Any| {
                    let result = match engine.downcast_ref::<E>() {
                        Some(engine) => {
                            catch_unwind(AssertUnwindSafe(|| f(engine))).map_err(panicked)
                        }
                        None => Err(BridgeError::EngineGone),
                    };
                    let _ = answer.send(result);
                });
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

    /// An engine that is not `Send`: only a pinned runner can hold it.
    struct Local {
        on: ThreadId,
        _not_send: Rc<()>,
    }

    impl Engine for Local {
        type Library = u64;
        const THREADS: ThreadModel = ThreadModel::Pinned;
        const MANIFESTS: ManifestFormat = ManifestFormat {
            suffix: ".local.toml",
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
        Runner::pinned("bridge-test", move || Local::start(&config)).expect("started")
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
}
