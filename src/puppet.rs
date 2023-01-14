use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::path::PathBuf;
use std::process::{self, Command};
use std::process::{Child, ExitStatus};

use tempfile::{tempdir, TempDir};

use crate::error::Error;
use crate::routes::CaptureOptions;

pub struct Puppet {
    pub id: i32,
    proc: Child,
    pub stdout: String,
    pub stderr: String,
}

impl Puppet {
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.proc.wait()
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        self.proc.kill()?;
        self.proc.wait()?;
        Ok(())
    }

    pub fn pid(&self) -> u32 {
        self.proc.id()
    }
}

pub struct Stdio {
    stdio: process::Stdio,
    label: String,
}

impl Stdio {
    pub const INHERITED: &str = "inherited";

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

pub struct PuppetManager {
    cur_id: i32,
    pups: HashMap<i32, Puppet>,
    out_dir: TempDir,
}

impl PuppetManager {
    pub fn new() -> Result<Self, Error> {
        Ok(PuppetManager {
            cur_id: 0,
            pups: HashMap::new(),
            out_dir: tempdir()?,
        })
    }

    pub fn push(
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
            stdout: stdout_label,
            stderr: stderr_label,
        };
        self.pups.insert(next_id, pup);
        self.cur_id += 1;
        return Ok(self.pups.get(&next_id).unwrap());
    }

    pub fn get(&mut self, id: i32) -> Option<&mut Puppet> {
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
                label: PathBuf::from(&stdout_filepath) // TODO: Exercise - Maybe can avoid the copy?
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
