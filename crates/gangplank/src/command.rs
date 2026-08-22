//! A shell command run every `period`, its output kept for the view. `lsof
//! -i`, `git status --porcelain`, `docker ps`: the source behind every
//! "dashboard" app. Runs on the background executor; each run waits for the
//! previous to finish, so a slow command stretches the period rather than
//! piling up.

use gpui::{App, Context, ElementId, Entity, Task, Window};
use std::panic::Location;
use std::process::Command;
use std::time::Duration;

/// One finished run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// The entity `use_command` hands back.
pub struct CommandState {
    last: Option<Output>,
    runs: u64,
    _task: Task<()>,
}

impl CommandState {
    /// The most recent run, `None` before the first finishes.
    pub fn output(&self) -> Option<&Output> {
        self.last.as_ref()
    }

    /// Completed runs so far; key a [`use_resource`](crate::use_resource) on it to post-process output.
    pub fn runs(&self) -> u64 {
        self.runs
    }
}

/// Run `program` with `args` every `period`, identified by the caller's source
/// location. The first run starts at once. Program, args, and period are fixed
/// at first render. Call only during render.
#[track_caller]
pub fn use_command(
    window: &mut Window,
    cx: &mut App,
    program: &str,
    args: &[&str],
    period: Duration,
) -> Entity<CommandState> {
    use_keyed_command(ElementId::CodeLocation(*Location::caller()), window, cx, program, args, period)
}

/// [`use_command`] with an explicit id.
pub fn use_keyed_command(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    program: &str,
    args: &[&str],
    period: Duration,
) -> Entity<CommandState> {
    let program = program.to_string();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    window.use_keyed_state(id, cx, |_, cx: &mut Context<CommandState>| CommandState {
        last: None,
        runs: 0,
        _task: cx.spawn(async move |this, cx| {
            loop {
                let (program, args) = (program.clone(), args.clone());
                let output = cx.background_executor().spawn(async move { run(&program, &args) }).await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.last = Some(output);
                        this.runs += 1;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(period).await;
            }
        }),
    })
}

fn run(program: &str, args: &[String]) -> Output {
    match Command::new(program).args(args).output() {
        Ok(out) => Output {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            success: out.status.success(),
        },
        Err(err) => Output { stdout: String::new(), stderr: err.to_string(), success: false },
    }
}
