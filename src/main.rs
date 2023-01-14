use rocket::http::ContentType;
use rocket::response;
use rocket::response::{Responder, Response};
use rocket::serde::json::{self, Json};
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::State;
use std::io::Cursor;

use std::collections::HashMap;
use std::process::ExitStatus;
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
struct CreateReq<'r> {
    exec: &'r str,
    args: Vec<&'r str>,
    capture: Option<CaptureOptions>,
}

// TODO: Can we remove the serde()?
#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct CreateResp {
    id: i32,
}

impl CreateResp {
    fn id(id: i32) -> CreateResp {
        CreateResp { id }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("filler error")]
    Foo(String),
    #[error("filler error")]
    PuppetNotFound(i32),
    #[error("io error")]
    IOError(#[from] std::io::Error),
    #[error("unknown error")]
    Unknown { source: std::io::Error },
}

#[derive(Serialize, Deserialize)]
struct ErrorJSONResp {
    err: String,
}

impl<'r> Responder<'r, 'r> for Error {
    fn respond_to(self, request: &'r rocket::Request<'_>) -> rocket::response::Result<'r> {
        let err_resp = ErrorJSONResp {
            err: format!("{:?}", self),
        };
        match json::to_string(&err_resp) {
            Ok(err_json) => Response::build()
                .header(ContentType::JSON)
                .sized_body(err_json.len(), Cursor::new(err_json))
                .ok(),
            Err(err) => response::Debug(err).respond_to(request),
        }
    }
}

const NO_ID: i32 = -1;

#[put("/cmd", format = "json", data = "<pup_req>")]
async fn cmd(
    pup_req: Json<CreateReq<'_>>,
    pups: &'_ State<Mutex<PuppetMap>>,
) -> Result<Json<CreateResp>, Error> {
    let (stdout_cfg, stderr_cfg) = pup_req.capture.unwrap_or(CaptureOptions::default()).stdio();
    let proc = Command::new(pup_req.exec)
        .args(&pup_req.args)
        .stdout(stdout_cfg)
        .stderr(stderr_cfg)
        .spawn()?;
    let mut pups = pups.lock().await;
    let cmd_id = pups.push(proc);
    Ok(Json(CreateResp::id(cmd_id)))
}

#[derive(Serialize, Deserialize)]
struct WaitResp {
    id: i32,
    exit_code: i32,
    signal_code: i32,
    signaled: bool,
    success: bool,
    err: Option<String>,
}

#[post("/wait/<id>")]
async fn wait(id: i32, pups: &'_ State<Mutex<PuppetMap>>) -> Result<Json<WaitResp>, Error> {
    let mut pups = pups.lock().await;
    if let Some(pup) = pups.get(id) {
        let exit_status = pup.wait()?;
        Ok(Json(WaitResp {
            id: pup.id,
            exit_code: exit_status.code().unwrap(),
            signal_code: NO_ID,
            signaled: false,
            success: exit_status.success(),
            err: None,
        }))
    } else {
        Err(Error::PuppetNotFound(id))
    }
}

struct Puppet {
    id: i32,
    proc: Child,
}

impl Puppet {
    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.proc.wait()
    }
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

    fn get(&mut self, id: i32) -> Option<&mut Puppet> {
        self.pups.get_mut(&id)
    }
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(Mutex::new(PuppetMap::new()))
        .mount("/", routes![cmd])
        .mount("/", routes![wait])
}

#[cfg(test)]
mod tests {
    use crate::{CreateReq, CreateResp};

    use super::rocket;
    use rocket::local::blocking::Client;

    fn get_rocket_client() -> Client {
        Client::tracked(rocket()).unwrap()
    }

    mod subtests {
        use crate::WaitResp;

        use super::*;

        #[test]
        fn test_run_cmd_successfully() {
            let client = get_rocket_client();
            let create_resp = client
                .put("/cmd")
                .json(&CreateReq {
                    exec: "echo",
                    args: vec!["foo"],
                    capture: None,
                })
                .dispatch()
                .into_json::<CreateResp>()
                .expect("expected non-None response for creating command");
            assert_eq!(create_resp.id, 0);
            let wait_resp = client
                .post(format!("/wait/{}", create_resp.id))
                .dispatch()
                .into_json::<WaitResp>()
                .expect("expected a non-None response for waiting on command");
            assert!(wait_resp.success);
        }
    }
}
