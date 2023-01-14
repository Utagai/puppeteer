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
    exit_code: i32,
    signal_code: i32,
    signaled: bool,
    pub success: bool,
    err: Option<String>,
}

// TODO: Do something about this wack shit.
const NO_ID: i32 = -1;

#[post("/wait/<id>")]
pub async fn wait(id: i32, pups: &'_ State<Mutex<PuppetManager>>) -> Result<Json<WaitResp>, Error> {
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
            err: None,
        }))
    } else {
        Err(Error::PuppetNotFound(id))
    }
}
