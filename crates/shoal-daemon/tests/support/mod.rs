use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use shoal_proto::{Message, Response, RpcError};
use shoal_testkit::{Connection, Received, TempEnv, eventually};

pub const SHOALD: &str = env!("CARGO_BIN_EXE_shoald");

pub fn fixture(close_on_exit: bool) -> TempEnv {
    let env = TempEnv::new();
    fs::create_dir_all(env.config_home().join("shoal")).unwrap();
    fs::write(
        env.config_home().join("shoal/config.toml"),
        config(close_on_exit),
    )
    .unwrap();
    fs::write(
        env.home().join(".bashrc"),
        "PS1='fixture> '; unset PROMPT_COMMAND\n",
    )
    .unwrap();
    env
}

pub fn config(close_on_exit: bool) -> String {
    format!(
        "[shell]\nprogram = '/bin/bash'\nintegration = 'off'\n[prompt]\nmode = 'shell'\n[behavior]\nclose_pane_on_exit = {close_on_exit}\n"
    )
}

pub struct Rpc {
    inner: Connection,
    next: u64,
    pub notices: Vec<Value>,
    pub changes: Vec<Value>,
}

impl Rpc {
    pub fn new(env: &TempEnv) -> Self {
        Self::hello(env, "cli", json!({}))
    }

    pub fn hello(env: &TempEnv, client: &str, variables: Value) -> Self {
        let mut rpc = Self {
            inner: env.connect(),
            next: 1,
            notices: Vec::new(),
            changes: Vec::new(),
        };
        rpc.ok("hello", json!({"protocol": 1, "client": client, "version": "0.1.0", "core": "different-core", "env": variables}));
        rpc
    }

    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        self.inner.send_request(id, method, params);
        id
    }

    pub fn response(&mut self, id: u64) -> Result<Value, RpcError> {
        loop {
            match self.inner.recv() {
                Received::Json(Message::Response(Response::Success { id: got, result })) => {
                    assert_eq!(got, id.into());
                    return Ok(result);
                }
                Received::Json(Message::Response(Response::Error { id: got, error })) => {
                    assert_eq!(got, Some(id.into()));
                    return Err(error);
                }
                Received::Json(Message::Notification(notification)) => {
                    self.record(notification.method, notification.params)
                }
                other => panic!("expected response {id}, got {other:?}"),
            }
        }
    }

    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        self.response(id)
            .unwrap_or_else(|error| panic!("{method}: {error:?}"))
    }

    pub fn error(&mut self, method: &str, params: Value, code: i32) {
        let id = self.send(method, params);
        let error = self.response(id).expect_err(method);
        assert_eq!(error.code, code, "{method}: {error:?}");
    }

    pub fn notification(&mut self, method: &str) -> Value {
        loop {
            match self.inner.recv() {
                Received::Json(Message::Notification(notification)) => {
                    let result = notification.params.clone();
                    let found = notification.method == method;
                    self.record(notification.method, notification.params);
                    if found {
                        return result;
                    }
                }
                other => panic!("expected {method}, got {other:?}"),
            }
        }
    }

    fn record(&mut self, method: String, params: Value) {
        match method.as_str() {
            "daemon.notice" => self.notices.push(params),
            "workspace.changed" => self.changes.push(params),
            _ => panic!("unexpected notification {method}"),
        }
    }

    pub fn workspace(&mut self) -> Value {
        let state = self.ok("workspace.get", json!({}));
        let workspace: shoal_model::Workspace =
            serde_json::from_value(state["workspace"].clone()).unwrap();
        shoal_model::validate(&workspace).unwrap();
        assert_eq!(state["rev"], state["workspace"]["rev"]);
        state["workspace"].clone()
    }
}

pub fn state(env: &TempEnv) -> PathBuf {
    env.state_home().join("shoal")
}

pub fn scrollback(env: &TempEnv, pane: &Value) -> PathBuf {
    state(env).join(format!(
        "panes/{}/scrollback.vt.zst",
        pane.as_str().unwrap()
    ))
}

pub fn saved_text(path: &Path) -> String {
    String::from_utf8(zstd::decode_all(fs::File::open(path).unwrap()).unwrap()).unwrap()
}

pub fn saved_workspace(env: &TempEnv) -> Value {
    serde_json::from_slice(&fs::read(state(env).join("workspace.json")).unwrap()).unwrap()
}

pub fn screen(rpc: &mut Rpc, pane: &Value) -> String {
    rpc.ok(
        "pane.capture",
        json!({"pane": pane, "format": "text", "scrollback": true}),
    )["data"]
        .as_str()
        .unwrap()
        .to_owned()
}

pub fn wait_for(rpc: &mut Rpc, pane: &Value, needle: &str) {
    eventually(&format!("{needle:?} on screen"), || {
        screen(rpc, pane).contains(needle).then_some(())
    });
}

/// A new session's pane, once its prompt is up.
pub fn session(rpc: &mut Rpc) -> Value {
    let session = rpc.ok("session.create", json!({}))["session"].clone();
    let workspace = rpc.workspace();
    let pane = workspace["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == session)
        .unwrap()["tabs"][0]["focused"]
        .clone();
    wait_for(rpc, &pane, "fixture>");
    pane
}

/// Print `marker` and wait for it. The command splits the marker, so only
/// its output contains it.
pub fn print(rpc: &mut Rpc, pane: &Value, marker: &str) {
    let (head, tail) = marker.split_at(3);
    rpc.ok(
        "pane.send_text",
        json!({"pane": pane, "text": format!("printf '{head}%s\\n' {tail}\n")}),
    );
    wait_for(rpc, pane, &format!("{marker}\n"));
}
