use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::State;

use thiserror::Error;

use std::collections::HashMap;
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
            CaptureOptions::flag_to_stdio(self.stdout),
            CaptureOptions::flag_to_stdio(self.stderr),
        )
    }

    fn flag_to_stdio(flag: bool) -> Stdio {
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
    pups: &'_ State<Mutex<PuppetMap>>,
) -> Result<Json<CreatePuppetResp>, PuppetError> {
    let (stdout_cfg, stderr_cfg) = pup_req.capture.unwrap_or(CaptureOptions::default()).stdio();
    let proc = Command::new(pup_req.exec)
        .args(&pup_req.args)
        .stdout(stdout_cfg)
        .stderr(stderr_cfg)
        .spawn()?;
    let mut pups = pups.lock().await;
    let cmd_id = pups.push(proc);
    Ok(Json(CreatePuppetResp { id: cmd_id }))
}

struct Puppet {
    id: i32,
    proc: Child,
}

struct PuppetMap {
    cur_id: i32,
    pups: HashMap<i32, Puppet>,
}

impl PuppetMap {
    fn new() -> Self {
        PuppetMap {
            cur_id: 0,
            pups: HashMap::new(),
        }
    }

    fn push(&mut self, cmd: Child) -> i32 {
        let next_id = self.cur_id;
        self.pups.insert(
            next_id,
            Puppet {
                id: next_id,
                proc: cmd,
            },
        );
        self.cur_id += 1;
        return next_id;
    }
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(Mutex::new(PuppetMap::new()))
        .mount("/", routes![index])
        .mount("/", routes![cmd])
}
