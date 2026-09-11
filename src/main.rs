//! commit-review: human review window before a commit started by Claude Code.
//!
//! Usage:
//!   commit-review hook                Claude Code PreToolUse hook: reads the
//!                                     event JSON on stdin, answers on stdout
//!   commit-review [--command <cmd>]   manual launch: exit 0 = accept,
//!                                     exit 10 = deny; the notes on stdout

mod diff;
mod git;
mod message;
mod state;
mod text;
mod ui;

use std::io::{Read, Write};

/// Exit code of a manual launch when the reviewer denies the commit.
const EXIT_DENIED: i32 = 10;

/// How a decision leaves the process.
#[derive(Clone, Copy)]
enum Output {
    /// Claude Code hook protocol: a JSON deny on stdout, exit 0 either way.
    Hook,
    /// Manual launch: the reason on stdout, exit 10 on deny.
    Plain,
}

/// A file of the diff with what an earlier attempt left on it.
struct Change {
    diff: diff::FileDiff,
    /// Marked viewed earlier and unchanged since.
    viewed: bool,
    restored: Vec<state::Restored>,
}

/// What the summary shows.
struct Context {
    repo: String,
    branch: String,
    /// The reviewer, from git config, for the comment boxes.
    user: String,
    /// The command the agent is about to run, when known.
    command: Option<String>,
    message: Option<message::CommitMessage>,
    /// What the message contains that deserves a look, per field.
    findings: Option<Findings>,
    /// The commit being rewritten by `--amend`, if any.
    amend: Option<Amend>,
    scope: message::Scope,
}

struct Findings {
    subject: Vec<message::Finding>,
    body: Vec<message::Finding>,
}

struct Amend {
    /// Short hash and subject of HEAD.
    head: String,
    /// Files HEAD already touches, from `git show --stat`.
    stat: String,
    /// The message is taken over from a commit, not given on the command line.
    message_kept: bool,
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("hook") => hook(),
        _ => review(flag_value("command"), Output::Plain),
    }
}

fn hook() -> ! {
    // A panic must still deny: Claude Code treats an unexpected exit code as
    // a non-blocking hook error and lets the commit through.
    std::panic::set_hook(Box::new(|info| {
        deny(Output::Hook, &format!("commit-review crashed: {info}"))
    }));
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("hook event on stdin");
    let event: serde_json::Value = serde_json::from_str(&input).expect("hook event is JSON");
    let command = event["tool_input"]["command"].as_str().unwrap_or("");
    if !message::is_git_commit(command) {
        std::process::exit(0);
    }
    if let Some(cwd) = event["cwd"].as_str() {
        std::env::set_current_dir(cwd).expect("hook cwd exists");
    }
    review(Some(command.to_string()), Output::Hook)
}

fn deny(output: Output, reason: &str) -> ! {
    finish(output, false, reason)
}

/// Leaves the process with the decision. The text reaches the agent either
/// way: as the deny reason, or as extra context on an accepted commit.
fn finish(output: Output, accept: bool, text: &str) -> ! {
    let line = match output {
        Output::Plain => text.to_string(),
        Output::Hook if accept && text.is_empty() => String::new(),
        Output::Hook => {
            let mut fields = serde_json::json!({ "hookEventName": "PreToolUse" });
            if accept {
                fields["permissionDecision"] = "allow".into();
                fields["additionalContext"] = text.into();
            } else {
                fields["permissionDecision"] = "deny".into();
                fields["permissionDecisionReason"] = text.into();
            }
            serde_json::json!({ "hookSpecificOutput": fields }).to_string()
        }
    };
    if !line.is_empty() {
        println!("{line}");
    }
    let _ = std::io::stdout().flush();
    let code = match (output, accept) {
        (Output::Plain, false) => EXIT_DENIED,
        _ => 0,
    };
    std::process::exit(code)
}

/// Value of `--<name> <value>` or `--<name>=<value>` on the command line.
fn flag_value(name: &str) -> Option<String> {
    let flag = format!("--{name}");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
        if let Some(v) = a.strip_prefix(&flag).and_then(|v| v.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

/// A manual launch has no command: show the whole working tree.
fn scope_of(command: Option<&str>) -> message::Scope {
    command.map_or(message::Scope::Worktree, message::scope)
}

/// What the window shows.
fn context(command: Option<&str>) -> Result<Context, String> {
    let mut message = command.and_then(message::extract);
    let mut amend = None;
    if command.is_some_and(message::amends) {
        let message_kept = message.is_none();
        if message_kept {
            // Without -m, git keeps the message of HEAD or of the -C revision.
            let rev = command
                .and_then(message::reused_message_rev)
                .unwrap_or_else(|| "HEAD".to_string());
            message = message::from_raw(&git::run(&["log", "-1", "--format=%B", &rev])?);
        }
        amend = Some(Amend {
            head: git::run(&["log", "-1", "--format=%h %s"])?,
            stat: git::run(&["show", "--stat", "--format=", "HEAD"])?,
            message_kept,
        });
    }
    let findings = message.as_ref().map(|m| Findings {
        subject: message::findings(&m.subject),
        body: message::findings(&m.body),
    });
    Ok(Context {
        repo: git::run(&["rev-parse", "--show-toplevel"])?,
        branch: git::run(&["branch", "--show-current"]).unwrap_or_default(),
        user: git::run(&["config", "user.name"]).unwrap_or_else(|_| "You".to_string()),
        scope: scope_of(command),
        command: command.map(str::to_string),
        message,
        findings,
        amend,
    })
}

/// The changes the commit will contain, with the earlier review of each.
fn changes(command: Option<&str>) -> Result<Vec<Change>, String> {
    let files = diff::changes(scope_of(command), command.is_some_and(message::amends))?;
    let saved = state::State::load()?;
    Ok(files
        .into_iter()
        .map(|diff| {
            let (viewed, restored) = saved.restore(&diff);
            Change { diff, viewed, restored }
        })
        .collect())
}

/// Leaves with the reviewer's decision and the text for the agent.
/// `reviews` is absent when the diff was never opened: the saved state
/// then stands as it is.
fn decide(
    output: Output,
    accept: bool,
    text: &str,
    reviews: Option<(&[diff::FileDiff], Vec<state::FileReview>)>,
) -> ! {
    let kept = match (accept, reviews) {
        (true, _) => state::State::clear(),
        (false, Some((files, reviews))) => state::State::build(files, reviews).save(),
        (false, None) => Ok(()),
    };
    // Losing the review state is not worth losing the decision.
    if let Err(e) = kept {
        eprintln!("commit-review: {e}");
    }
    finish(output, accept, text.trim())
}

fn review(command: Option<String>, output: Output) -> ! {
    // Git paths are shown relative to the root; the cwd may be deeper.
    if let Ok(root) = git::run(&["rev-parse", "--show-toplevel"]) {
        std::env::set_current_dir(root).expect("repository root exists");
    }
    // Every decision leaves the process from inside the window: `run`
    // returning means the window closed without one. A gate that accepts by
    // accident is worthless.
    match ui::run(command, output) {
        Ok(()) => deny(output, "Review window closed without a decision: commit denied."),
        Err(e) => deny(output, &format!("Review window failed to open, commit denied: {e}")),
    }
}
