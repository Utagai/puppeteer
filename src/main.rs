use rocket::tokio::sync::Mutex;

use crate::puppet::PuppetManager;
use crate::routes::{cmd, kill, wait};

#[macro_use]
extern crate rocket;

mod error;
mod puppet;
mod routes;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(Mutex::new(
            PuppetManager::new().expect("failed to start up puppet manager"),
        ))
        .mount("/", routes![cmd])
        .mount("/", routes![wait])
        .mount("/", routes![kill])
}

#[cfg(test)]
mod tests {
    use crate::routes::{CaptureOptions, CreateReq, CreateResp, WaitResp};

    use super::rocket;
    use core::time;
    use rocket::{http::Status, local::blocking::Client};
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    fn get_rocket_client() -> Client {
    const INHERITED: &str = "inherited";

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
            assert_eq!(create_resp.stdout, INHERITED);
        }

        if capture_opts.stderr {
            assert!(create_resp.stderr != "");
            output.stderr = get_contents(&create_resp.stderr);
        } else {
            assert_eq!(create_resp.stderr, INHERITED);
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

    fn kill_id(client: &Client, id: i32) {
        assert_eq!(
            client.post(format!("/kill/{}", id)).dispatch().status(),
            Status::Ok
        )
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
        assert_eq!(create_resp.stdout, INHERITED);
        assert_eq!(create_resp.stderr, INHERITED);
        assert_ne!(create_resp.pid, 0);
        let wait_resp = wait_for_id(&client, create_resp.id);
        assert!(wait_resp.success);
    }

    #[test]
    fn check_wait_resp_fields() {
        let client = get_rocket_client();
        let create_resp = create_req(&client, "echo", vec!["-n", ""], CaptureOptions::none());
        let wait_resp = wait_for_id(&client, create_resp.id);
        assert!(wait_resp.success);
        assert_eq!(wait_resp.exit_code, 0);
        assert!(!wait_resp.signaled);
        assert_eq!(!wait_resp.signal_code, -1);
    }

    // TODO: Need to test error cases:
    // * Puppet that DNE.
    // * double-wait
    // * double-kill
    // * wait after kill
    // * exec that DNE
    // * exec that isn't an exec?

    fn find_proc(pid: u32) -> Option<psutil::process::Process> {
        psutil::process::processes()
            .expect("failed to get a listing of system processes")
            .into_iter()
            .find(|proc_res| proc_res.as_ref().map_or(false, |proc| proc.pid() == pid)) // Option<Result<ProcessResult<Process>>>
            .map(|proc_res| proc_res.map_or(None, |proc| Some(proc))) // Option<Option<Process>>
            .map(|proc| proc.expect("wtf")) // Option<Process>
    }

    #[test]
    fn kill_cmd() {
        let client = get_rocket_client();
        let forever = get_testscript_path("forever.sh");
        let create_resp = create_req(
            &client,
            forever
                .to_str()
                .expect("failed to unwrap forever script filepath"),
            vec![],
            CaptureOptions::none(),
        );
        assert_ne!(find_proc(create_resp.pid), None);
        kill_id(&client, create_resp.id);
        println!("ok killed {}", create_resp.pid);
        while find_proc(create_resp.pid) != None {}
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
        assert_eq!(create_resp.stderr, INHERITED);

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
        // Let's clean-up by killing that script we ran, since it'll otherwise run for a really long time:
        kill_id(&client, create_resp.id);
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
