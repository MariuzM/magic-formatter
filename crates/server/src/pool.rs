use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Sender, unbounded};
use lsp_server::{ErrorCode, Message, RequestId, Response};
use serde_json::Value;

type Job = Box<dyn FnOnce() + Send>;

type Inflight = Arc<Mutex<HashMap<RequestId, bool>>>;

pub struct Pool {
    jobs:     Sender<Job>,
    sender:   Sender<Message>,
    inflight: Inflight,
}

impl Pool {
    pub fn new(sender: Sender<Message>) -> Self {
        let (jobs, queue) = unbounded::<Job>();
        let workers       = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 8);
        for _ in 0..workers {
            let queue = queue.clone();
            std::thread::spawn(move || {
                for job in queue {
                    job();
                }
            });
        }
        Self {
            jobs,
            sender,
            inflight: Inflight::default(),
        }
    }

    pub fn spawn(&self, id: RequestId, work: impl FnOnce(&Sender<Message>) -> Value + Send + 'static) {
        self.inflight.lock().unwrap().insert(id.clone(), false);
        let (sender, inflight) = (self.sender.clone(), self.inflight.clone());
        let job                = move || {
            let cancelled = inflight.lock().unwrap().get(&id).copied().unwrap_or(false);
            let response  = if cancelled {
                Response::new_err(id.clone(), ErrorCode::RequestCanceled as i32, "cancelled".into())
            } else {
                match catch_unwind(AssertUnwindSafe(|| work(&sender))) {
                    Ok(result) => Response::new_ok(id.clone(), result),
                    Err(_) => Response::new_err(id.clone(), ErrorCode::InternalError as i32, "magic-formatter: internal error".into()),
                }
            };
            inflight.lock().unwrap().remove(&id);
            sender.send(response.into()).ok();
        };
        self.jobs.send(Box::new(job)).ok();
    }

    pub fn cancel(&self, id: &RequestId) {
        if let Some(cancelled) = self.inflight.lock().unwrap().get_mut(id) {
            *cancelled = true;
        }
    }
}
