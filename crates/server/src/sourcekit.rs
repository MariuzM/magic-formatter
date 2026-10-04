use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(4);

const INIT_TIMEOUT: Duration = Duration::from_secs(30);

type Pending = Arc<Mutex<HashMap<RequestId, Sender<Response>>>>;

pub struct SourceKit {
    outgoing: Sender<Message>,
    pending:  Pending,
    next_id:  AtomicI32,
    alive:    Arc<AtomicBool>,
    child:    Mutex<Child>,
}

fn command() -> Command {
    if cfg!(target_os = "macos") {
        let mut cmd = Command::new("xcrun");
        cmd.arg("sourcekit-lsp");
        cmd
    } else {
        Command::new("sourcekit-lsp")
    }
}

fn is_empty(result: &Value) -> bool {
    match result {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => {
            let empty_list  = ["items", "signatures"].iter().any(|k| o.get(*k).and_then(Value::as_array).is_some_and(Vec::is_empty));
            let empty_hover = o.get("contents").is_some_and(|c| c.is_null() || c == "" || c["value"] == "");
            empty_list || empty_hover
        }
        _ => false,
    }
}

fn reply(req: &Request) -> Response {
    let result = match req.method.as_str() {
        "workspace/configuration" => json!(vec![Value::Null; req.params["items"].as_array().map_or(0, Vec::len)]),
        _ => Value::Null,
    };
    Response::new_ok(req.id.clone(), result)
}

fn write(stdin: &mut ChildStdin, msg: Message) -> bool {
    msg.write(stdin).is_ok()
}

fn handshake(stdin: &mut ChildStdin, pending: &Pending, init: Value) -> bool {
    let id       = RequestId::from(0);
    let (tx, rx) = bounded(1);
    pending.lock().unwrap().insert(id.clone(), tx);
    write(stdin, Request::new(id, "initialize".into(), init).into())
        && rx.recv_timeout(INIT_TIMEOUT).is_ok()
        && write(stdin, Notification::new("initialized".into(), json!({})).into())
}

impl SourceKit {
    pub fn spawn(init: Value) -> Option<Self> {
        let mut child                                               = command().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let mut stdin                                               = child.stdin.take()?;
        let stdout                                                  = child.stdout.take()?;
        let pending                                                 = Pending::default();
        let alive                                                   = Arc::new(AtomicBool::new(true));
        let (outgoing, queue): (Sender<Message>, Receiver<Message>) = unbounded();

        let (reader_pending, reader_alive, replies) = (pending.clone(), alive.clone(), outgoing.clone());
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = Message::read(&mut reader) {
                match msg {
                    Message::Response(res) => {
                        if let Some(tx) = reader_pending.lock().unwrap().remove(&res.id) {
                            tx.send(res).ok();
                        }
                    }
                    Message::Request(req) => {
                        replies.send(reply(&req).into()).ok();
                    }
                    Message::Notification(_) => {}
                }
            }
            reader_alive.store(false, Ordering::SeqCst);
            reader_pending.lock().unwrap().clear();
        });

        let (writer_pending, writer_alive) = (pending.clone(), alive.clone());
        std::thread::spawn(move || {
            if handshake(&mut stdin, &writer_pending, init) {
                for msg in queue {
                    if !write(&mut stdin, msg) {
                        break;
                    }
                }
            }
            writer_alive.store(false, Ordering::SeqCst);
            writer_pending.lock().unwrap().clear();
        });

        Some(Self {
            outgoing,
            pending,
            next_id: AtomicI32::new(1),
            alive,
            child: Mutex::new(child),
        })
    }

    pub fn notify(&self, method: &str, params: Value) {
        if self.alive.load(Ordering::SeqCst) {
            self.outgoing.send(Notification::new(method.into(), params).into()).ok();
        }
    }

    pub fn request(&self, method: &str, mut params: Value) -> Option<Value> {
        if !self.alive.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(p) = params.as_object_mut() {
            p.remove("workDoneToken");
            p.remove("partialResultToken");
        }
        let id       = RequestId::from(self.next_id.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = bounded(1);
        self.pending.lock().unwrap().insert(id.clone(), tx);
        self.outgoing.send(Request::new(id.clone(), method.into(), params).into()).ok()?;
        match rx.recv_timeout(TIMEOUT) {
            Ok(res) => res.response_result.ok().filter(|r| !is_empty(r)),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.notify("$/cancelRequest", json!({ "id": id }));
                None
            }
        }
    }
}

impl Drop for SourceKit {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}
