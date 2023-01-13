use rocket::http::Status;
use rocket::response::status;
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

#[derive(Serialize, Deserialize, Copy, Clone)]
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

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct CreatePuppetReq<'r> {
    exec: &'r str,
    args: Vec<&'r str>,
    capture: Option<CaptureOptions>,
}

// TODO: Can we remove the serde()?
#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct CreatePuppetResp {
    id: i32,
    err: Option<String>,
}

impl CreatePuppetResp {
    fn id(id: i32) -> CreatePuppetResp {
        CreatePuppetResp { id, err: None }
    }

    fn err(errmsg: &str) -> CreatePuppetResp {
        CreatePuppetResp {
            id: NO_ID,
            err: Some(errmsg.to_owned()),
        }
    }
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

const NO_ID: i32 = -1;

#[put("/cmd", format = "json", data = "<pup_req>")]
async fn cmd(
    pup_req: Json<CreatePuppetReq<'_>>,
    pups: &'_ State<Mutex<PuppetMap>>,
) -> status::Custom<Json<CreatePuppetResp>> {
    let (stdout_cfg, stderr_cfg) = pup_req.capture.unwrap_or(CaptureOptions::default()).stdio();
    let proc_res = Command::new(pup_req.exec)
        .args(&pup_req.args)
        .stdout(stdout_cfg)
        .stderr(stderr_cfg)
        .spawn();
    match proc_res {
        Ok(proc) => {
            let mut pups = pups.lock().await;
            let cmd_id = pups.push(proc);
            status::Custom(Status::Accepted, Json(CreatePuppetResp::id(cmd_id)))
        }
        Err(err) => status::Custom(
            Status::BadRequest,
            Json(CreatePuppetResp::err(&format!("{:?}", err))),
        ),
    }
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
        .mount("/", routes![cmd])
}

#[cfg(test)]
mod tests {
    use crate::{CreatePuppetReq, CreatePuppetResp};

    use super::rocket;
    use rocket::local::blocking::Client;

    fn get_rocket_client() -> Client {
        Client::tracked(rocket()).unwrap()
    }

    mod subtests {
        use super::*;

        #[test]
        fn test_run_cmd_successfully() {
            let client = get_rocket_client();
            let resp = client
                .put("/cmd")
                .json(&CreatePuppetReq {
                    exec: "echo",
                    args: vec!["foo"],
                    capture: None,
                })
                .dispatch()
                .into_json::<CreatePuppetResp>()
                .expect("expected non-None response for creating command");
            assert_eq!(resp.err, None);
            assert_eq!(resp.id, 0);
        }
    }
}
