use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::State;

#[macro_use]
extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[derive(Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct CaptureOptions {
    stdout: bool,
    stderr: bool,
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

#[put("/cmd", format = "json", data = "<command>")]
async fn cmd(
    command: Json<CreatePuppetReq<'_>>,
    queue: &'_ State<Mutex<PuppetQueue>>,
) -> Json<CreatePuppetResp> {
    let mut queue = queue.lock().await;
    let cmd_id = queue.push(Puppet::from(&command));
    return Json(CreatePuppetResp { id: cmd_id });
}
}

struct Puppet {
    id: i32,
    exec: String,
    args: Vec<String>,
    capture: CaptureOptions,
}

impl From<&Json<CreatePuppetReq<'_>>> for Puppet {
    fn from(req: &Json<CreatePuppetReq>) -> Self {
        Puppet {
            id: 0,
            exec: req.exec.to_owned(),
            args: (&req.args)
                .into_iter()
                .map(|v| v.to_owned().to_owned())
                .collect(),
            capture: req
                .capture
                .clone()
                .or(Some(CaptureOptions::default()))
                .unwrap(),
        }
    }
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

    fn push(&mut self, pup: Puppet) -> i32 {
        let next_id = self.cur_id;
        self.pups.push(pup);
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
