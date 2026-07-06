use alloc::boxed::Box;
use alloc::vec::Vec;
use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};
use spin::Mutex;

pub mod executor;
pub mod keyboard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        TaskId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct Task {
    id: TaskId,
    // "+ Send" (Fase 17): precisa pra Task poder viver dentro do static
    // Mutex de PENDING_SPAWNS abaixo (Mutex<T> so e' Sync se T: Send).
    // Todo Future criado neste kernel e' Send de qualquer jeito (nao ha
    // thread real, so estado normal), entao nao restringe nada na pratica.
    future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Task {
    pub fn new(future: impl Future<Output = ()> + Send + 'static) -> Task {
        Task {
            id: TaskId::new(),
            future: Box::pin(future),
        }
    }

    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}

/// Fila de Tasks recem-criadas esperando pra entrar no executor (Fase 17,
/// comando `spawn` do shell). O `Executor` em si e' local a `kernel_main`
/// (nao um singleton) -- se fosse exposto direto via Mutex global, um
/// `poll()` que tentasse chamar `executor.spawn(...)` enquanto o proprio
/// `run_ready_tasks` esta com `&mut self` do executor emprestado pra
/// aquele mesmo poll deadlockaria (Mutex nao e' reentrante). Essa fila
/// separada, leve, evita isso: `queue_spawn` so empurra a Task; quem
/// dreha e' o laco do `Executor::run` (`src/task/executor.rs`).
static PENDING_SPAWNS: Mutex<Vec<Task>> = Mutex::new(Vec::new());

pub fn queue_spawn(task: Task) {
    PENDING_SPAWNS.lock().push(task);
}

pub(crate) fn drain_pending_spawns() -> Vec<Task> {
    let mut guard = PENDING_SPAWNS.lock();
    core::mem::take(&mut *guard)
}
