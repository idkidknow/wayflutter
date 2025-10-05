use std::{convert::Infallible, pin::Pin, time::Duration};

use anyhow::Result;
use futures::{StreamExt, channel::mpsc};
use smol::LocalExecutor;

use crate::FlutterEngine;

type NormalTask = Box<dyn FnOnce(&FlutterEngine) + Send + 'static>;

pub trait AsyncTask {
    fn run<'a>(&mut self, engine: &'a FlutterEngine) -> Pin<Box<dyn Future<Output = ()> + 'a>>;
}

/// Trait object cannot consume self, so we take the actual AsyncFnOnce from Some(f).
impl<F> AsyncTask for Option<F>
where
    F: AsyncFnOnce(&FlutterEngine) + 'static,
{
    fn run<'a>(&mut self, engine: &'a FlutterEngine) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        if let Some(f) = self.take() {
            Box::pin(f(engine))
        } else {
            Box::pin(futures::future::ready(()))
        }
    }
}

enum Task
where
    Self: Send,
{
    Normal(NormalTask),
    Async(Box<dyn AsyncTask + Send>),
}

#[derive(Clone)]
pub struct TaskRunnerHandle
where
    Self: Sync,
{
    tx: mpsc::UnboundedSender<Task>,
}

impl TaskRunnerHandle {
    pub fn post_task(
        &self,
        task: impl FnOnce(&FlutterEngine) + Send + 'static,
    ) -> Result<()> {
        let ret = self.tx.unbounded_send(Task::Normal(Box::new(task)));
        match ret {
            Ok(()) => Ok(()),
            Err(_) => Err(anyhow::anyhow!("Failed to post task"))?,
        }
    }

    pub fn post_async_task(
        &self,
        task: impl AsyncFnOnce(&FlutterEngine) + Send + 'static,
    ) -> Result<()> {
        let ret = self.tx.unbounded_send(Task::Async(Box::new(Some(task))));
        match ret {
            Ok(()) => Ok(()),
            Err(_) => Err(anyhow::anyhow!("Failed to post async task"))?,
        }
    }

    pub fn post_task_after(
        &self,
        task: impl FnOnce(&FlutterEngine) + Send + 'static,
        delay: Duration,
    ) -> Result<()> {
        if delay.is_zero() {
            self.post_task(task)?;
        } else {
            self.post_async_task(async move |engine| {
                smol::Timer::after(delay).await;
                task(engine);
            })?;
        }
        Ok(())
    }
}

pub fn make_task_runner<'a>(
    engine: &'a FlutterEngine,
) -> (
    impl Future<Output = Result<Infallible>> + 'a,
    TaskRunnerHandle,
) {
    let ex = LocalExecutor::new();
    let (tx, rx) = mpsc::unbounded::<Task>();

    let runner = async move {
        let receiving = async {
            let mut rx = rx;
            while let Some(task) = rx.next().await {
                match task {
                    Task::Normal(task) => {
                        task(engine);
                    }
                    Task::Async(mut task) => {
                        ex.spawn(task.run(engine)).detach();
                    }
                }
            }
            anyhow::bail!("all task senders dropped");
        };

        ex.run(receiving).await
    };

    (runner, TaskRunnerHandle { tx })
}
