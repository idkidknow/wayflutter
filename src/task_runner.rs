use std::{cell::UnsafeCell, collections::BinaryHeap, time::Duration};

use anyhow::Result;
use futures::{
    FutureExt, SinkExt, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};

use crate::{error::FFIFlutterEngineResultExt, ffi, FlutterEngine};

pub struct PendingTask {
    pub task: ffi::FlutterTask,
    pub target_nanos: u64,
}

impl PartialEq for PendingTask {
    fn eq(&self, other: &Self) -> bool {
        self.target_nanos == other.target_nanos
    }
}

impl PartialOrd for PendingTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.target_nanos.partial_cmp(&self.target_nanos)
    }
}

impl Eq for PendingTask {}

impl Ord for PendingTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.target_nanos.cmp(&self.target_nanos)
    }
}

pub struct TaskRunnerData {
    pub tx: UnboundedSender<PendingTask>,
    pub main_thread: std::thread::ThreadId,
}

impl TaskRunnerData {
    pub fn new_on_current_thread(tx: UnboundedSender<PendingTask>) -> Self {
        Self {
            tx,
            main_thread: std::thread::current().id(),
        }
    }
}

pub fn run_task_runner<'a>(
    engine: &'a FlutterEngine,
    rx: UnboundedReceiver<PendingTask>,
) -> impl Future<Output = Result<()>> + 'a {
    let queue = UnsafeCell::new(BinaryHeap::<PendingTask>::new());
    let (earlier_task_pushed_tx, earlier_task_pushed) = futures::channel::mpsc::channel::<()>(1);

    async move {
        let receiving = async {
            let mut rx = rx;
            let mut earlier_task_pushed_tx = earlier_task_pushed_tx;
            while let Some(task) = rx.next().await {
                let now = unsafe { ffi::FlutterEngineGetCurrentTime() };
                let target = task.target_nanos;
                let delay = target.saturating_sub(now);
                if delay == 0 {
                    unsafe {
                        ffi::FlutterEngineRunTask(engine.engine, &task.task)
                            .into_flutter_engine_result()?;
                    }
                } else {
                    // SAFETY: single-threaded, no reentrancy and no acrossing await point
                    let earlier: bool = unsafe {
                        let earlier = match (*queue.get()).peek() {
                            Some(task) => task.target_nanos > target,
                            None => true,
                        };
                        (*queue.get()).push(task);
                        earlier
                    };
                    if earlier {
                        earlier_task_pushed_tx.send(()).await?;
                    }
                }
            }
            anyhow::Ok(())
        };

        let executing = async {
            let mut earlier_task_pushed = earlier_task_pushed;
            loop {
                // SAFETY: single-threaded, no reentrancy and no acrossing await point
                let delay_nanos: u64 = unsafe {
                    let queue = &mut *queue.get();

                    let mut ret = u64::MAX;
                    while let Some(task) = queue.peek() {
                        let now = ffi::FlutterEngineGetCurrentTime();
                        let delay = task.target_nanos.saturating_sub(now);
                        if delay == 0 {
                            ffi::FlutterEngineRunTask(engine.engine, &task.task);
                            queue.pop();
                        } else {
                            ret = delay;
                            break;
                        }
                    }
                    ret
                };

                let timer = smol::Timer::after(Duration::from_nanos(delay_nanos));
                let earlier_task_pushed = earlier_task_pushed.next();

                futures::select! {
                    _ = futures::FutureExt::fuse(timer) => (),
                    _ = earlier_task_pushed.fuse() => (),
                };
            }
        };

        futures::select! {
            result = receiving.fuse() => result,
            _ = executing.fuse() => unreachable!(),
        }
    }
}
