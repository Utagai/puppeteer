use rocket::response::{self, Responder};
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::Request;
use rocket::State;

use thiserror::Error;

use std::process::{Child, Command, Stdio};

#[macro_use]
extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[derive(Deserialize, Copy, Clone)]
#[serde(crate = "rocket::serde")]
struct CaptureOptions {
    stdout: bool,
    stderr: bool,
}

impl CaptureOptions {
    fn stdio(&self) -> (Stdio, Stdio) {
        (
            CaptureOptions::flagToStdio(self.stdout),
            CaptureOptions::flagToStdio(self.stderr),
        )
    }

    fn flagToStdio(flag: bool) -> Stdio {
        if flag {
            Stdio::piped()
        } else {
            Stdio::inherit()
        }
    }
}

impl Default for CaptureOptions {
    fn default() -> CaptureOptions {
        CaptureOptions {
            stdout: false,
            stderr: false,
        }
    }
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct CreatePuppetReq<'r> {
    exec: &'r str,
    args: Vec<&'r str>,
    capture: Option<CaptureOptions>,
}

// TODO: Can we remove the serde()?
#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
struct CreatePuppetResp {
    id: i32,
}

#[derive(Error, Debug, Responder)]
pub enum PuppetError {
    #[error("filler error")]
    Foo(String),
    #[error("io error")]
    IOError(#[from] std::io::Error),
    #[error("unknown error")]
    Unknown { source: std::io::Error },
}

#[put("/cmd", format = "json", data = "<pup_req>")]
async fn cmd(
    pup_req: Json<CreatePuppetReq<'_>>,
    queue: &'_ State<Mutex<PuppetQueue>>,
) -> Result<Json<CreatePuppetResp>, PuppetError> {
    let (stdout_cfg, stderr_cfg) = pup_req.capture.unwrap_or(CaptureOptions::default()).stdio();
    let proc = Command::new(pup_req.exec)
        .args(&pup_req.args)
        .stdout(stdout_cfg)
        .stderr(stderr_cfg)
        .spawn()?;
    let mut queue = queue.lock().await;
    let cmd_id = queue.push(proc);
    Ok(Json(CreatePuppetResp { id: cmd_id }))
}

struct Puppet {
    id: i32,
    proc: Child,
}

struct PuppetQueue {
    cur_id: i32,
    pups: Vec<Puppet>,
}

impl PuppetQueue {
    fn new() -> Self {
        PuppetQueue {
            cur_id: 0,
            pups: Vec::new(),
        }
    }

    fn push(&mut self, cmd: Child) -> i32 {
        let next_id = self.cur_id;
        self.pups.push(Puppet { id: next_id, proc: cmd });
        self.cur_id += 1;
        return next_id;
    }
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(Mutex::new(PuppetQueue::new()))
        .mount("/", routes![index])
        .mount("/", routes![cmd])
}
