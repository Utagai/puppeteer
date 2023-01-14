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
use std::process::{self, Child, Command};

#[macro_use]
extern crate rocket;

#[derive(Serialize, Deserialize, Copy, Clone)]
struct CaptureOptions {
    stdout: bool,
    stderr: bool,
}

impl CaptureOptions {
    #[allow(dead_code)]
    fn all() -> CaptureOptions {
        CaptureOptions {
            stdout: true,
            stderr: true,
        }
    }

    #[allow(dead_code)]
    fn stdout() -> CaptureOptions {
        CaptureOptions {
            stdout: true,
            stderr: false,
        }
    }

    #[allow(dead_code)]
    fn stderr() -> CaptureOptions {
        CaptureOptions {
            stdout: false,
            stderr: true,
        }
    }

    fn none() -> CaptureOptions {
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
struct CreateReq<'r> {
    exec: &'r str,
    args: Vec<&'r str>,
    capture: Option<CaptureOptions>,
}

#[derive(Serialize, Deserialize)]
struct CreateResp {
    id: i32,
    stdout: String,
    stderr: String,
}

impl From<&Puppet> for CreateResp {
    fn from(value: &Puppet) -> Self {
        CreateResp {
            id: value.id,
            stdout: value.stdout_filepath.clone(),
            stderr: value.stderr_filepath.clone(),
        }
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
    let pup = pups.push(
        pup_req.exec,
        &pup_req.args,
        pup_req.capture.unwrap_or(CaptureOptions::default()),
    )?;
    Ok(Json(CreateResp::from(pup)))
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

struct Stdio {
    stdio: process::Stdio,
    label: String,
}

impl Stdio {
    const INHERITED: &str = "inherited";

    fn inherit() -> Stdio {
        Stdio {
            stdio: process::Stdio::inherit(),
            label: String::from(Stdio::INHERITED),
        }
    }
}

impl Into<process::Stdio> for Stdio {
    fn into(self) -> process::Stdio {
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
    ) -> Result<&Puppet, Error> {
        let next_id = self.cur_id;
        let (stdout, stderr) = self.make_stdio(next_id, capture_opts)?;
        // TODO: Exercise - Can we avoid the copy here?
        let (stdout_label, stderr_label) = (stdout.label.clone(), stderr.label.clone());
        let proc = Command::new(exec)
            .args(args)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?;
        let pup = Puppet {
            id: next_id,
            proc,
            stdout_filepath: stdout_label,
            stderr_filepath: stderr_label,
        };
        self.pups.insert(next_id, pup);
        self.cur_id += 1;
        return Ok(self.pups.get(&next_id).unwrap());
    }

    fn get(&mut self, id: i32) -> Option<&mut Puppet> {
        self.pups.get_mut(&id)
    }

    fn make_stdio(&self, id: i32, capture_opts: CaptureOptions) -> Result<(Stdio, Stdio), Error> {
        let dirpath = self.out_dir.path();
        let id_dir = dirpath.join(id.to_string());
        create_dir_all(&id_dir)?;
        let stdout_file = if capture_opts.stdout {
            let stdout_filepath = id_dir.join("stdout");
            Stdio {
                stdio: process::Stdio::from(File::create(&stdout_filepath)?),
                label: PathBuf::from(&stdout_filepath) // TODO: Maybe can avoid the copy.
                    .to_str()
                    .expect("failed to convert Path -> &str")
                    .to_string(),
            }
        } else {
            Stdio::inherit()
        };
        let stderr_file = if capture_opts.stderr {
            let stderr_filepath = id_dir.join("stderr");
            Stdio {
                stdio: process::Stdio::from(File::create(&stderr_filepath)?),
                label: stderr_filepath
                    .to_str()
                    .expect("failed to convert Path -> &str")
                    .to_string(),
            }
        } else {
            Stdio::inherit()
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
    use crate::{CaptureOptions, CreateReq, CreateResp, Stdio, WaitResp};

    use super::rocket;
    use core::time;
    use rocket::local::blocking::Client;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    fn get_rocket_client() -> Client {
        Client::tracked(rocket()).unwrap()
    }

    fn create_req(
        client: &Client,
        exec: &str,
        args: Vec<&str>,
        capture: CaptureOptions,
    ) -> CreateResp {
        client
            .put("/cmd")
            .json(&CreateReq {
                exec,
                args,
                capture: Some(capture),
            })
            .dispatch()
            .into_json::<CreateResp>()
            .expect("expected non-None response for creating command")
    }

    struct StdOutput {
        stdout: String,
        stderr: String,
    }

    fn run_cmd_and_get_output(
        client: &Client,
        exec: &str,
        args: Vec<&str>,
        capture_opts: CaptureOptions,
    ) -> StdOutput {
        let create_resp = create_req(client, exec, args, capture_opts);
        let wait_resp = wait_for_id(&client, create_resp.id);
        assert!(wait_resp.success);
        let mut output = StdOutput {
            stdout: String::from(""),
            stderr: String::from(""),
        };
        if capture_opts.stdout {
            assert!(create_resp.stdout != "");
            output.stdout = get_contents(&create_resp.stdout);
        } else {
            assert_eq!(create_resp.stdout, Stdio::INHERITED);
        }

        if capture_opts.stderr {
            assert!(create_resp.stderr != "");
            output.stderr = get_contents(&create_resp.stderr);
        } else {
            assert_eq!(create_resp.stderr, Stdio::INHERITED);
        }

        output
    }

    fn wait_for_id(client: &Client, id: i32) -> WaitResp {
        client
            .post(format!("/wait/{}", id))
            .dispatch()
            .into_json::<WaitResp>()
            .expect("expected a non-None response for waiting on command")
    }

    fn get_contents(filepath: &str) -> String {
        std::fs::read_to_string(&filepath)
            .expect(&format!("failed to open stdout file @ {}", filepath))
    }

    fn get_testscript_path<P: AsRef<Path>>(name: P) -> PathBuf {
        let current_dir = std::env::current_dir().expect("failed to get current working directory");
        return current_dir.join("testscripts").join(name);
    }

    #[test]
    fn run_cmd_successfully() {
        let client = get_rocket_client();
        let create_resp = create_req(&client, "echo", vec!["-n", ""], CaptureOptions::none());
        assert_eq!(create_resp.id, 0);
        assert_eq!(create_resp.stdout, Stdio::INHERITED);
        assert_eq!(create_resp.stderr, Stdio::INHERITED);
        let wait_resp = wait_for_id(&client, create_resp.id);
        assert!(wait_resp.success);
    }

    #[test]
    fn cmd_inherits_from_server_env() {
        let client = get_rocket_client();
        let expected_env_var_key = format!("puppet-{}", Uuid::new_v4());
        let expected_env_var_val = "blah";
        std::env::set_var(&expected_env_var_key, expected_env_var_val);
        let output =
            run_cmd_and_get_output(&client, "env", vec![], CaptureOptions::stdout()).stdout;
        output.contains(&format!(
            "{}={}",
            expected_env_var_key, expected_env_var_val
        ));
    }

    #[test]
    fn can_stream_cmd_output_without_wait() {
        let client = get_rocket_client();

        let periodic_print = get_testscript_path("periodic.sh");
        let create_resp = create_req(
            &client,
            periodic_print
                .to_str()
                .expect("failed to unwrap periodic script filepath"),
            vec!["bar"],
            CaptureOptions::stdout(),
        );
        assert_eq!(create_resp.id, 0);
        assert!(create_resp.stdout != "");
        assert_eq!(create_resp.stderr, Stdio::INHERITED);

        let get_last_num = || loop {
            let contents = get_contents(&create_resp.stdout);
            if contents.len() > 0 {
                let last_line = contents
                    .split("\n")
                    .last()
                    .expect("expected a non-zero length periodic output to have a last line");
                if let Ok(last_num) = last_line.parse::<i32>() {
                    break last_num;
                } else {
                    // It is possible that we end up picking up the
                    // very first line of the file, which would be an
                    // empty line with only a newline. It is fairly
                    // rare, but possible as long as the threads align
                    // properly.
                    continue;
                }
            }
        };

        // The logic is as follows, given that the script is just outputting a monotonically increasing integer every second:
        //	1. Keep the loop going until it finds any amount of output.
        //	2. Once output is found, find the last line of that output, and save it.
        //  3. Run a loop again, repeatedly finding the last line.
        //  4. Keep doing this until you find a last-line that shows a number greater than the one saved in step 2.
        // This proves that we are finding data that is being continuously streamed.
        let last_num = get_last_num();

        const DELAY: time::Duration = time::Duration::from_millis(100);
        const MAX_ATTEMPTS: i32 = 100; // delay * max_attempts = 10 seconds. Should be more than enough.
        let mut attempts = 0;
        while get_last_num() == last_num {
            std::thread::sleep(DELAY);
            attempts += 1;
            assert!(attempts < MAX_ATTEMPTS);
        }

        // If we get here, we found a differing number -- we've passed.
    }

    mod captures {
        use super::*;

        #[test]
        fn stdout() {
            let client = get_rocket_client();
            let expected_output = "bar";
            assert_eq!(
                run_cmd_and_get_output(
                    &client,
                    "echo",
                    vec![expected_output],
                    CaptureOptions::stdout()
                )
                .stdout,
                format!("{}\n", expected_output)
            );
        }

        #[test]
        fn stderr() {
            let client = get_rocket_client();
            let expected_output = "bar";
            let stderr_print = get_testscript_path("stderr.sh");
            assert_eq!(
                run_cmd_and_get_output(
                    &client,
                    stderr_print
                        .to_str()
                        .expect("failed to unwrap stderr script filepath"),
                    vec![expected_output],
                    CaptureOptions::stderr()
                )
                .stderr,
                format!("{}\n", expected_output)
            );
        }

        #[test]
        fn both() {
            let client = get_rocket_client();
            // TODO: we should maybe emit two different values for stdout v stderr -- this tests we aren't mixing the two up.
            let expected_output = "bar";
            let both_std_print = get_testscript_path("both_std.sh");
            let output = run_cmd_and_get_output(
                &client,
                both_std_print
                    .to_str()
                    .expect("failed to unwrap stderr script filepath"),
                vec![expected_output],
                CaptureOptions::all(),
            );
            assert_eq!(output.stdout, format!("{}\n", expected_output));
            assert_eq!(output.stderr, format!("{}\n", expected_output));
        }
    }
}
