use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::tokio::sync::Mutex;
use rocket::State;

use crate::error::Error;
use crate::puppet::{Puppet, PuppetManager};

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct CaptureOptions {
    pub stdout: bool,
    pub stderr: bool,
}

impl CaptureOptions {
    #[allow(dead_code)]
    pub fn all() -> CaptureOptions {
        CaptureOptions {
            stdout: true,
            stderr: true,
        }
    }

    #[allow(dead_code)]
    pub fn stdout() -> CaptureOptions {
        CaptureOptions {
            stdout: true,
            stderr: false,
        }
    }

    #[allow(dead_code)]
    pub fn stderr() -> CaptureOptions {
        CaptureOptions {
            stdout: false,
            stderr: true,
        }
    }

    pub fn none() -> CaptureOptions {
        CaptureOptions {
            stdout: false,
            stderr: false,
        }
    }
}

impl Default for CaptureOptions {
    fn default() -> CaptureOptions {
        CaptureOptions::none()
    }
}

#[derive(Serialize, Deserialize)]
pub struct CreateReq<'r> {
    pub exec: &'r str,
    pub args: Vec<&'r str>,
    pub capture: Option<CaptureOptions>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateResp {
    pub id: i32,
    pub pid: u32,
    pub stdout: String,
    pub stderr: String,
}

impl From<&Puppet> for CreateResp {
    fn from(pup: &Puppet) -> Self {
        CreateResp {
            id: pup.id,
            pid: pup.pid(),
            // TODO: Exercise - Can we avoid clone()?
            stdout: pup.stdout.clone(),
            stderr: pup.stderr.clone(),
        }
    }
}

#[put("/cmd", format = "json", data = "<pup_req>")]
pub async fn cmd(
    pup_req: Json<CreateReq<'_>>,
    pups: &'_ State<Mutex<PuppetManager>>,
) -> Result<Json<CreateResp>, Error> {
    let mut pups = pups.lock().await;
    let pup = pups.push(
        pup_req.exec,
        &pup_req.args,
        pup_req.capture.unwrap_or(CaptureOptions::default()),
    )?;
    Ok(Json(CreateResp::from(pup)))
}

#[derive(Serialize, Deserialize)]
pub struct WaitResp {
    id: i32,
    pub exit_code: i32,
    pub signal_code: i32,
    pub signaled: bool,
    pub success: bool,
}

impl WaitResp {
    const NOVAL: i32 = -1;

    fn from(id: i32, status: ExitStatus) -> Self {
        let resp = WaitResp {
            id,
            exit_code: status.code().unwrap_or(Self::NOVAL),
            signal_code: status.code().unwrap_or(
                status
                    .signal()
                    .unwrap_or(status.stopped_signal().unwrap_or(Self::NOVAL)),
            ),
            signaled: status.code().is_none(),
            success: status.success(),
        };

        resp
    }
}

#[post("/wait/<id>")]
pub async fn wait(id: i32, pups: &'_ State<Mutex<PuppetManager>>) -> Result<Json<WaitResp>, Error> {
    let mut pups = pups.lock().await;
    if let Some(pup) = pups.get(id) {
        let exit_status = pup.wait()?;
        Ok(Json(WaitResp::from(pup.id, exit_status)))
    } else {
        Err(Error::PuppetNotFound(id))
    }
}

#[post("/kill/<id>")]
pub async fn kill(id: i32, pups: &'_ State<Mutex<PuppetManager>>) -> Result<Status, Error> {
    let mut pups = pups.lock().await;
    if let Some(pup) = pups.get(id) {
        pup.kill()?;
        Ok(Status::Ok)
    } else {
        // TODO: Can we use ? and avoid if let else stuff?
        Err(Error::PuppetNotFound(id))
    }
}
