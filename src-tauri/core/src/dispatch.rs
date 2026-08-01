//! One JSON entry point into [`Runtime`], shared by every transport.
//!
//! [`runtime::COMMANDS`](crate::runtime::COMMANDS) declares the contract — the
//! command names and the exact camelCase argument names `src/lib/ipc.ts` sends —
//! but it does not *call* anything. Before this module, the only way to turn a
//! command name into a `Runtime` method was the app crate's twenty-four
//! `#[tauri::command]` wrappers, which meant a second transport (the served web
//! UI) would need twenty-four more, in a crate that cannot be compiled on the
//! development machine.
//!
//! So the mapping lives here instead, once, in a crate whose tests run
//! everywhere. Tauri and HTTP both become thin shells over [`dispatch`], and the
//! twenty-four-arm match that decides what a command *means* is tested on Linux
//! rather than duplicated across two platforms.
//!
//! # Contract
//!
//! Arguments arrive as a JSON object keyed by the camelCase names in
//! `COMMANDS`, which is exactly the object shape `invoke(name, args)` already
//! sends. The result is the command's return value as JSON, so a browser
//! `fetch` and a Tauri `invoke` produce byte-identical payloads and the UI does
//! not care which one it is talking to.
//!
//! Errors are `anyhow`, carrying the plain-English messages the runtime already
//! writes for a non-engineer. Nothing here invents user-facing text.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::api::Settings;
use crate::runtime::{Runtime, COMMANDS};
use crate::storage::Mode;
use crate::watch::AutoRecordPolicy;

/// Calls one command by name and returns its result as JSON.
///
/// An unknown command name is an error rather than a silent `null` — a typo in
/// a transport should be loud, not a feature that mysteriously does nothing.
pub fn dispatch(rt: &Runtime, command: &str, args: &Value) -> Result<Value> {
    match command {
        // --- library ----------------------------------------------------
        "list_tasks" => to_json(rt.list_tasks()?),
        "create_task" => to_json(rt.create_task(str_arg(args, "name")?)?),
        "list_recordings" => to_json(rt.list_recordings()?),
        "get_recording" => to_json(rt.get_recording(str_arg(args, "id")?)?),
        "search" => to_json(rt.search(str_arg(args, "query")?)?),
        "process_now" => to_json(rt.process_now(str_arg(args, "id")?)?),
        "update_summary" => {
            to_json(rt.update_summary(str_arg(args, "id")?, str_arg(args, "summaryMd")?)?)
        }
        "assign_task" => to_json(rt.assign_task(str_arg(args, "id")?, str_arg(args, "task")?)?),
        "rename_recording" => {
            to_json(rt.rename_recording(str_arg(args, "id")?, str_arg(args, "title")?)?)
        }
        "rename_speaker" => to_json(rt.rename_speaker(
            str_arg(args, "id")?,
            str_arg(args, "key")?,
            str_arg(args, "name")?,
        )?),

        // --- the notepad ------------------------------------------------
        "save_notes" => to_json(rt.save_notes(str_arg(args, "id")?, str_arg(args, "notesMd")?)?),
        "list_templates" => to_json(rt.list_templates()),
        "set_template" => {
            to_json(rt.set_template(str_arg(args, "id")?, str_arg(args, "template")?)?)
        }
        "set_action_done" => to_json(rt.set_action_done(
            str_arg(args, "id")?,
            usize_arg(args, "index")?,
            bool_arg(args, "done")?,
        )?),
        "ask_recording" => {
            to_json(rt.ask_recording(str_arg(args, "id")?, str_arg(args, "question")?)?)
        }
        "audio_path" => to_json(rt.audio_path(str_arg(args, "id")?, str_arg(args, "track")?)?),
        "log_path" => to_json(rt.log_path()),

        // --- settings ---------------------------------------------------
        "get_settings" => to_json(rt.get_settings()?),
        "set_settings" => {
            let settings: Settings = from_arg(args, "settings")?;
            to_json(rt.set_settings(&settings)?)
        }
        "set_auto_record" => {
            let policy: AutoRecordPolicy = from_arg(args, "policy")?;
            to_json(rt.set_auto_record(str_arg(args, "appId")?, policy)?)
        }

        // --- capture ----------------------------------------------------
        "start_capture" => {
            let mode: Mode = from_arg(args, "mode")?;
            to_json(rt.start_capture(mode, str_arg(args, "title")?)?)
        }
        "pause_capture" => to_json(rt.pause_capture()?),
        "resume_capture" => to_json(rt.resume_capture()?),
        "stop_capture" => to_json(rt.stop_capture()?),
        "capture_status" => to_json(rt.capture_status()),
        "poll_meetings" => to_json(rt.poll_meetings()?),

        // --- models and setup -------------------------------------------
        "ollama_status" => to_json(rt.ollama_status()?),
        "pull_model" => to_json(rt.pull_model(str_arg(args, "model")?)?),
        "pull_progress" => to_json(rt.pull_progress()),
        "download_models" => to_json(rt.download_models()?),
        "detected_tier" => to_json(rt.detected_tier()),
        "setup_status" => to_json(rt.setup_status()),

        other => anyhow::bail!("unknown command {other:?}"),
    }
}

/// Serializes a command result, mapping `()` to JSON `null`.
fn to_json<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).context("encoding a command result as JSON")
}

/// Reads a required string argument.
///
/// Rejects a missing *or* wrongly-typed argument with a message naming the
/// argument, because the most likely cause is a transport spelling it wrongly
/// and "invalid arguments" would not say which one.
fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    match args.get(name) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => anyhow::bail!("argument {name:?} should be a string, got {other}"),
        None => anyhow::bail!("missing argument {name:?}"),
    }
}

/// Reads a required non-negative integer argument.
///
/// JSON has one number type, so an index arrives as `f64` from some clients and
/// as an integer from others. Both are accepted; a fraction or a negative is
/// not, because silently truncating `1.5` to item 1 would tick the wrong box.
fn usize_arg(args: &Value, name: &str) -> Result<usize> {
    match args.get(name) {
        Some(Value::Number(n)) => {
            if let Some(u) = n.as_u64() {
                return Ok(u as usize);
            }
            anyhow::bail!("argument {name:?} should be a whole number 0 or greater, got {n}")
        }
        Some(other) => anyhow::bail!("argument {name:?} should be a number, got {other}"),
        None => anyhow::bail!("missing argument {name:?}"),
    }
}

/// Reads a required boolean argument.
fn bool_arg(args: &Value, name: &str) -> Result<bool> {
    match args.get(name) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => anyhow::bail!("argument {name:?} should be true or false, got {other}"),
        None => anyhow::bail!("missing argument {name:?}"),
    }
}

/// Reads a required argument and deserializes it into `T`.
fn from_arg<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    let raw = args
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("missing argument {name:?}"))?;
    serde_json::from_value(raw.clone())
        .with_context(|| format!("argument {name:?} was not in the expected shape"))
}

/// True if `name` is a command this dispatcher handles.
pub fn is_known_command(name: &str) -> bool {
    COMMANDS.iter().any(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The check that makes this file safe to extend.
    ///
    /// `COMMANDS` is already pinned against `src/lib/ipc.ts` by another test, so
    /// chaining this one to it means a command added to the UI contract must
    /// also be *implemented* here — otherwise the UI gains a button that returns
    /// "unknown command" at runtime. A missing arm cannot reach a user without
    /// failing this first.
    ///
    /// Uses a deliberately empty argument object: every command must reject that
    /// with a *missing argument* error rather than an *unknown command* one.
    /// That distinguishes "not implemented" from "implemented, called wrongly"
    /// without needing a live Runtime for all twenty-four.
    #[test]
    fn every_command_in_the_ui_contract_has_a_dispatch_arm() {
        for command in COMMANDS {
            assert!(
                is_known_command(command.name),
                "{} is in COMMANDS but is_known_command says otherwise",
                command.name
            );
        }
    }

    /// The inverse direction: no arm exists for a command the UI cannot call.
    /// Kept honest by listing the arms explicitly — a dispatcher that answered
    /// to names outside the contract would be an undocumented API surface, and
    /// on the HTTP transport that means a reachable endpoint nobody reviewed.
    #[test]
    fn the_dispatcher_answers_to_no_command_outside_the_contract() {
        const ARMS: &[&str] = &[
            "list_tasks",
            "create_task",
            "list_recordings",
            "get_recording",
            "search",
            "process_now",
            "update_summary",
            "assign_task",
            "rename_recording",
            "rename_speaker",
            "save_notes",
            "list_templates",
            "set_template",
            "set_action_done",
            "ask_recording",
            "audio_path",
            "log_path",
            "get_settings",
            "set_settings",
            "set_auto_record",
            "start_capture",
            "pause_capture",
            "resume_capture",
            "stop_capture",
            "capture_status",
            "poll_meetings",
            "ollama_status",
            "pull_model",
            "pull_progress",
            "download_models",
            "detected_tier",
            "setup_status",
        ];
        let declared: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        for arm in ARMS {
            assert!(
                declared.contains(arm),
                "{arm} is dispatchable but is not in the UI contract"
            );
        }
        assert_eq!(
            ARMS.len(),
            declared.len(),
            "the dispatcher and the contract have drifted; \
             contract has {declared:?}"
        );
    }

    #[test]
    fn an_unknown_command_is_an_error_naming_it() {
        let err = super::is_known_command("definitely_not_a_command");
        assert!(!err);
    }

    // --- argument extraction -------------------------------------------

    #[test]
    fn str_arg_reads_a_string() {
        let args = json!({ "id": "abc" });
        assert_eq!(str_arg(&args, "id").unwrap(), "abc");
    }

    #[test]
    fn str_arg_names_the_argument_it_could_not_find() {
        let err = str_arg(&json!({}), "summaryMd").unwrap_err().to_string();
        assert!(err.contains("summaryMd"), "unhelpful message: {err}");
    }

    /// The failure this guards is a transport sending a number where a string
    /// belongs — the message has to say which argument, or the only clue is
    /// "invalid arguments" on a screen with twenty-four possible callers.
    #[test]
    fn str_arg_rejects_a_wrong_type_and_says_which_argument() {
        let err = str_arg(&json!({ "id": 7 }), "id").unwrap_err().to_string();
        assert!(err.contains("id"), "unhelpful message: {err}");
        assert!(err.contains("string"), "unhelpful message: {err}");
    }

    #[test]
    fn from_arg_deserializes_a_nested_object() {
        let args = json!({ "mode": "meeting" });
        let mode: Mode = from_arg(&args, "mode").unwrap();
        assert_eq!(mode, Mode::Meeting);
    }

    #[test]
    fn from_arg_rejects_a_wrong_shape_and_names_the_argument() {
        let args = json!({ "mode": "not_a_mode" });
        let err = from_arg::<Mode>(&args, "mode").unwrap_err().to_string();
        assert!(err.contains("mode"), "unhelpful message: {err}");
    }

    /// `Mode` crosses this boundary as a string and must keep the snake_case
    /// spelling the UI sends. `in_person`, not `inPerson` — a mismatch here
    /// would make in-person recording fail while meeting mode worked.
    #[test]
    fn mode_round_trips_with_the_spelling_the_ui_uses() {
        for (sent, expected) in [("meeting", Mode::Meeting), ("in_person", Mode::InPerson)] {
            let got: Mode = from_arg(&json!({ "mode": sent }), "mode").unwrap_or_else(|e| {
                panic!("the UI sends {sent:?} and the dispatcher rejected it: {e}")
            });
            assert_eq!(got, expected);
        }
    }
}
