use rocket::http::ContentType;
use rocket::response;
use rocket::response::{Responder, Response};
use rocket::serde::json::{self, Json};
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::State;
use tempfile::{tempdir, TempDir};

use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::Cursor;
use std::path::PathBuf;
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
    #[error("puppet with id {0} not found")]
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
    pups: &'_ State<Mutex<PuppetManager>>,
) -> Result<Json<CreateResp>, Error> {
    let mut pups = pups.lock().await;
    let cmd_id = pups.push(
        pup_req.exec,
        &pup_req.args,
        pup_req.capture.unwrap_or(CaptureOptions::default()),
    )?;
    Ok(Json(CreateResp::id(cmd_id)))
}

#[derive(Serialize, Deserialize)]
struct WaitResp {
    id: i32,
    exit_code: i32,
    signal_code: i32,
    signaled: bool,
    success: bool,
    stdout_file: String,
    stderr_file: String,
    err: Option<String>,
}

#[post("/wait/<id>")]
async fn wait(id: i32, pups: &'_ State<Mutex<PuppetManager>>) -> Result<Json<WaitResp>, Error> {
    let mut pups = pups.lock().await;
    if let Some(pup) = pups.get(id) {
        let exit_status = pup.wait()?;
        Ok(Json(WaitResp {
            id: pup.id,
            exit_code: exit_status.code().unwrap(),
            // TODO: Handle signals.
            signal_code: NO_ID,
            signaled: false,
            success: exit_status.success(),
            stdout_file: pup.stdout_filepath.clone(),
            stderr_file: pup.stderr_filepath.clone(),
            err: None,
        }))
    } else {
        Err(Error::PuppetNotFound(id))
    }
}

struct Puppet {
    id: i32,
    proc: Child,
    stdout_filepath: String,
    stderr_filepath: String,
}

impl Puppet {
    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.proc.wait()
    }
}

// TODO: We need to work on the names we use for these kinda things, e.g. make_stdio(), Outstdio, etc.
struct OutStdio {
    stdio: Stdio,
    label: String,
}

impl OutStdio {
    const INHERITED: &str = "inherited";

    fn inherit() -> OutStdio {
        OutStdio {
            stdio: Stdio::inherit(),
            label: String::from(OutStdio::INHERITED),
        }
    }
}

impl Into<Stdio> for OutStdio {
    fn into(self) -> Stdio {
        return self.stdio;
    }
}

struct PuppetManager {
    cur_id: i32,
    pups: HashMap<i32, Puppet>,
    out_dir: TempDir,
}

impl PuppetManager {
    fn new() -> Result<Self, Error> {
        Ok(PuppetManager {
            cur_id: 0,
            pups: HashMap::new(),
            out_dir: tempdir()?,
        })
    }

    fn push(
        &mut self,
        exec: &str,
        args: &Vec<&str>,
        capture_opts: CaptureOptions,
    ) -> Result<i32, Error> {
        let next_id = self.cur_id;
        let (stdout, stderr) = self.make_stdio(next_id, capture_opts)?;
        // TODO: Exercise - Can we avoid the copy here?
        let (stdout_label, stderr_label) = (stdout.label.clone(), stderr.label.clone());
        let proc = Command::new(exec)
            .args(args)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?;
        self.pups.insert(
            next_id,
            Puppet {
                id: next_id,
                proc,
                stdout_filepath: stdout_label,
                stderr_filepath: stderr_label,
            },
        );
        self.cur_id += 1;
        return Ok(next_id);
    }

    // TODO: Should have a non-mut option.
    fn get(&mut self, id: i32) -> Option<&mut Puppet> {
        self.pups.get_mut(&id)
    }

    fn make_stdio(
        &self,
        id: i32,
        capture_opts: CaptureOptions,
    ) -> Result<(OutStdio, OutStdio), Error> {
        let dirpath = self.out_dir.path();
        let id_dir = dirpath.join(id.to_string());
        create_dir_all(&id_dir)?;
        let stdout_file = if capture_opts.stdout {
            let stdout_filepath = id_dir.join("stdout");
            OutStdio {
                stdio: Stdio::from(File::create(&stdout_filepath)?),
                label: PathBuf::from(&stdout_filepath) // TODO: Maybe can avoid the copy.
                    .to_str()
                    .expect("failed to convert Path -> &str")
                    .to_string(),
            }
        } else {
            OutStdio::inherit()
        };
        let stderr_file = if capture_opts.stderr {
            let stderr_filepath = id_dir.join("stderr");
            OutStdio {
                stdio: Stdio::from(File::create(&stderr_filepath)?),
                label: stderr_filepath
                    .to_str()
                    .expect("failed to convert Path -> &str")
                    .to_string(),
            }
        } else {
            OutStdio::inherit()
        };
        Ok((stdout_file, stderr_file))
    }
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(Mutex::new(
            PuppetManager::new().expect("failed to start up puppet manager"),
        ))
        .mount("/", routes![cmd])
        .mount("/", routes![wait])
}

#[cfg(test)]
mod tests {
    use crate::{CreateReq, CreateResp, WaitResp};

    use super::rocket;
    use rocket::local::blocking::Client;

    fn get_rocket_client() -> Client {
        Client::tracked(rocket()).unwrap()
    }

    #[test]
    fn run_cmd_successfully() {
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
