//! Cortex API JSON-RPC 2.0 protocol definitions and request builders.
//!
//! The Emotiv Cortex service (shipped with the
//! [EMOTIV Launcher](https://www.emotiv.com/products/emotiv-launcher)) exposes a
//! JSON-RPC 2.0 API over a secure WebSocket at `wss://localhost:6868`.
//!
//! This module provides typed builder functions for every supported API method,
//! along with request-ID constants, warning/error codes, stream names, and
//! sampling-rate constants.
//!
//! You normally don't call these directly — [`CortexClient`](crate::client::CortexClient)
//! and [`CortexHandle`](crate::client::CortexHandle) use them internally. They
//! are public so you can construct custom requests if needed.

use serde_json::{json, Value};

// ── Request IDs ───────────────────────────────────────────────────────────────

pub const QUERY_HEADSET_ID: i64 = 1;
pub const CONNECT_HEADSET_ID: i64 = 2;
pub const REQUEST_ACCESS_ID: i64 = 3;
pub const AUTHORIZE_ID: i64 = 4;
pub const CREATE_SESSION_ID: i64 = 5;
pub const SUB_REQUEST_ID: i64 = 6;
pub const SETUP_PROFILE_ID: i64 = 7;
pub const QUERY_PROFILE_ID: i64 = 8;
pub const TRAINING_ID: i64 = 9;
pub const DISCONNECT_HEADSET_ID: i64 = 10;
pub const CREATE_RECORD_REQUEST_ID: i64 = 11;
pub const STOP_RECORD_REQUEST_ID: i64 = 12;
pub const EXPORT_RECORD_ID: i64 = 13;
pub const INJECT_MARKER_REQUEST_ID: i64 = 14;
pub const SENSITIVITY_REQUEST_ID: i64 = 15;
pub const MENTAL_COMMAND_ACTIVE_ACTION_ID: i64 = 16;
pub const MENTAL_COMMAND_BRAIN_MAP_ID: i64 = 17;
pub const MENTAL_COMMAND_TRAINING_THRESHOLD: i64 = 18;
pub const SET_MENTAL_COMMAND_ACTIVE_ACTION_ID: i64 = 19;
pub const HAS_ACCESS_RIGHT_ID: i64 = 20;
pub const GET_CURRENT_PROFILE_ID: i64 = 21;
pub const GET_CORTEX_INFO_ID: i64 = 22;
pub const UPDATE_MARKER_REQUEST_ID: i64 = 23;
pub const UNSUB_REQUEST_ID: i64 = 24;
pub const REFRESH_HEADSET_LIST_ID: i64 = 25;
pub const QUERY_RECORDS_ID: i64 = 26;
pub const REQUEST_DOWNLOAD_RECORDS_ID: i64 = 27;
pub const SYNC_WITH_HEADSET_CLOCK_ID: i64 = 28;

// ── Warning codes ─────────────────────────────────────────────────────────────

pub const CORTEX_STOP_ALL_STREAMS: i64 = 0;
pub const CORTEX_CLOSE_SESSION: i64 = 1;
pub const USER_LOGIN: i64 = 2;
pub const USER_LOGOUT: i64 = 3;
pub const ACCESS_RIGHT_GRANTED: i64 = 9;
pub const ACCESS_RIGHT_REJECTED: i64 = 10;
pub const PROFILE_LOADED: i64 = 13;
pub const PROFILE_UNLOADED: i64 = 14;
pub const CORTEX_AUTO_UNLOAD_PROFILE: i64 = 15;
pub const EULA_ACCEPTED: i64 = 17;
pub const CORTEX_RECORD_POST_PROCESSING_DONE: i64 = 30;
pub const HEADSET_DISCONNECTED: i64 = 102;
pub const HEADSET_CONNECTION_FAILED: i64 = 103;
pub const HEADSET_CONNECTED: i64 = 104;
pub const HEADSET_SCANNING_FINISHED: i64 = 142;

// ── Error codes ───────────────────────────────────────────────────────────────

pub const ERR_PROFILE_ACCESS_DENIED: i64 = -32046;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default WebSocket endpoint for the Cortex service.
pub const CORTEX_WS_URL: &str = "wss://localhost:6868";

/// EEG sample rate in Hz (128 Hz per channel for most Emotiv headsets).
pub const EEG_FREQUENCY: f64 = 128.0;

/// Available data stream names.
pub const STREAM_EEG: &str = "eeg";
pub const STREAM_MOT: &str = "mot";
pub const STREAM_DEV: &str = "dev";
pub const STREAM_MET: &str = "met";
pub const STREAM_POW: &str = "pow";
pub const STREAM_COM: &str = "com";
pub const STREAM_FAC: &str = "fac";
pub const STREAM_SYS: &str = "sys";

// ── Request builders ──────────────────────────────────────────────────────────

/// Build a JSON-RPC 2.0 request.
pub fn build_request(id: i64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

pub fn has_access_right(client_id: &str, client_secret: &str) -> Value {
    build_request(HAS_ACCESS_RIGHT_ID, "hasAccessRight", json!({
        "clientId": client_id,
        "clientSecret": client_secret
    }))
}

pub fn request_access(client_id: &str, client_secret: &str) -> Value {
    build_request(REQUEST_ACCESS_ID, "requestAccess", json!({
        "clientId": client_id,
        "clientSecret": client_secret
    }))
}

pub fn authorize(client_id: &str, client_secret: &str, license: &str, debit: i64) -> Value {
    build_request(AUTHORIZE_ID, "authorize", json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "license": license,
        "debit": debit
    }))
}

pub fn query_headsets() -> Value {
    build_request(QUERY_HEADSET_ID, "queryHeadsets", json!({}))
}

pub fn connect_headset(headset_id: &str) -> Value {
    build_request(CONNECT_HEADSET_ID, "controlDevice", json!({
        "command": "connect",
        "headset": headset_id
    }))
}

pub fn disconnect_headset(headset_id: &str) -> Value {
    build_request(DISCONNECT_HEADSET_ID, "controlDevice", json!({
        "command": "disconnect",
        "headset": headset_id
    }))
}

pub fn refresh_headset_list() -> Value {
    build_request(REFRESH_HEADSET_LIST_ID, "controlDevice", json!({
        "command": "refresh"
    }))
}

pub fn create_session(auth_token: &str, headset_id: &str) -> Value {
    build_request(CREATE_SESSION_ID, "createSession", json!({
        "cortexToken": auth_token,
        "headset": headset_id,
        "status": "active"
    }))
}

pub fn close_session(auth_token: &str, session_id: &str) -> Value {
    build_request(CREATE_SESSION_ID, "updateSession", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "status": "close"
    }))
}

pub fn subscribe(auth_token: &str, session_id: &str, streams: &[&str]) -> Value {
    build_request(SUB_REQUEST_ID, "subscribe", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "streams": streams
    }))
}

pub fn unsubscribe(auth_token: &str, session_id: &str, streams: &[&str]) -> Value {
    build_request(UNSUB_REQUEST_ID, "unsubscribe", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "streams": streams
    }))
}

pub fn create_record(auth_token: &str, session_id: &str, title: &str, description: &str) -> Value {
    build_request(CREATE_RECORD_REQUEST_ID, "createRecord", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "title": title,
        "description": description
    }))
}

pub fn stop_record(auth_token: &str, session_id: &str) -> Value {
    build_request(STOP_RECORD_REQUEST_ID, "stopRecord", json!({
        "cortexToken": auth_token,
        "session": session_id
    }))
}

pub fn export_record(
    auth_token: &str, folder: &str, format: &str,
    stream_types: &[&str], record_ids: &[&str], version: &str,
) -> Value {
    let mut params = json!({
        "cortexToken": auth_token,
        "folder": folder,
        "format": format,
        "streamTypes": stream_types,
        "recordIds": record_ids
    });
    if format == "CSV" {
        params["version"] = json!(version);
    }
    build_request(EXPORT_RECORD_ID, "exportRecord", params)
}

pub fn inject_marker(
    auth_token: &str, session_id: &str,
    time: f64, value: &str, label: &str,
) -> Value {
    build_request(INJECT_MARKER_REQUEST_ID, "injectMarker", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "time": time,
        "value": value,
        "label": label
    }))
}

pub fn update_marker(
    auth_token: &str, session_id: &str,
    marker_id: &str, time: f64,
) -> Value {
    build_request(UPDATE_MARKER_REQUEST_ID, "updateMarker", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "markerId": marker_id,
        "time": time
    }))
}

pub fn query_profile(auth_token: &str) -> Value {
    build_request(QUERY_PROFILE_ID, "queryProfile", json!({
        "cortexToken": auth_token
    }))
}

pub fn get_current_profile(auth_token: &str, headset_id: &str) -> Value {
    build_request(GET_CURRENT_PROFILE_ID, "getCurrentProfile", json!({
        "cortexToken": auth_token,
        "headset": headset_id
    }))
}

pub fn setup_profile(auth_token: &str, headset_id: &str, profile_name: &str, status: &str) -> Value {
    build_request(SETUP_PROFILE_ID, "setupProfile", json!({
        "cortexToken": auth_token,
        "headset": headset_id,
        "profile": profile_name,
        "status": status
    }))
}

pub fn train_request(
    auth_token: &str, session_id: &str,
    detection: &str, action: &str, status: &str,
) -> Value {
    build_request(TRAINING_ID, "training", json!({
        "cortexToken": auth_token,
        "detection": detection,
        "session": session_id,
        "action": action,
        "status": status
    }))
}

pub fn get_mental_command_active_action(auth_token: &str, profile_name: &str) -> Value {
    build_request(MENTAL_COMMAND_ACTIVE_ACTION_ID, "mentalCommandActiveAction", json!({
        "cortexToken": auth_token,
        "profile": profile_name,
        "status": "get"
    }))
}

pub fn set_mental_command_active_action(auth_token: &str, session_id: &str, actions: &[&str]) -> Value {
    build_request(SET_MENTAL_COMMAND_ACTIVE_ACTION_ID, "mentalCommandActiveAction", json!({
        "cortexToken": auth_token,
        "session": session_id,
        "status": "set",
        "actions": actions
    }))
}

pub fn get_mental_command_sensitivity(auth_token: &str, profile_name: &str) -> Value {
    build_request(SENSITIVITY_REQUEST_ID, "mentalCommandActionSensitivity", json!({
        "cortexToken": auth_token,
        "profile": profile_name,
        "status": "get"
    }))
}

pub fn set_mental_command_sensitivity(
    auth_token: &str, profile_name: &str, session_id: &str, values: &[i32],
) -> Value {
    build_request(SENSITIVITY_REQUEST_ID, "mentalCommandActionSensitivity", json!({
        "cortexToken": auth_token,
        "profile": profile_name,
        "session": session_id,
        "status": "set",
        "values": values
    }))
}

pub fn get_mental_command_brain_map(auth_token: &str, profile_name: &str, session_id: &str) -> Value {
    build_request(MENTAL_COMMAND_BRAIN_MAP_ID, "mentalCommandBrainMap", json!({
        "cortexToken": auth_token,
        "profile": profile_name,
        "session": session_id
    }))
}

pub fn get_mental_command_training_threshold(auth_token: &str, session_id: &str) -> Value {
    build_request(MENTAL_COMMAND_TRAINING_THRESHOLD, "mentalCommandTrainingThreshold", json!({
        "cortexToken": auth_token,
        "session": session_id
    }))
}

pub fn query_records(auth_token: &str, query: Value) -> Value {
    let mut params = json!({ "cortexToken": auth_token });
    if let Value::Object(map) = query {
        for (k, v) in map {
            params[k] = v;
        }
    }
    build_request(QUERY_RECORDS_ID, "queryRecords", params)
}

pub fn request_download_records(auth_token: &str, record_ids: &[&str]) -> Value {
    build_request(REQUEST_DOWNLOAD_RECORDS_ID, "requestToDownloadRecordData", json!({
        "cortexToken": auth_token,
        "recordIds": record_ids
    }))
}

pub fn sync_with_headset_clock(headset_id: &str) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    build_request(SYNC_WITH_HEADSET_CLOCK_ID, "syncWithHeadsetClock", json!({
        "headset": headset_id,
        "systemTime": now,
        "monotonicTime": now
    }))
}

pub fn get_cortex_info() -> Value {
    build_request(GET_CORTEX_INFO_ID, "getCortexInfo", json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_request_structure() {
        let req = build_request(42, "testMethod", json!({"key": "value"}));
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], 42);
        assert_eq!(req["method"], "testMethod");
        assert_eq!(req["params"]["key"], "value");
    }

    #[test]
    fn test_has_access_right() {
        let req = has_access_right("cid", "csecret");
        assert_eq!(req["method"], "hasAccessRight");
        assert_eq!(req["id"], HAS_ACCESS_RIGHT_ID);
        assert_eq!(req["params"]["clientId"], "cid");
        assert_eq!(req["params"]["clientSecret"], "csecret");
    }

    #[test]
    fn test_request_access() {
        let req = request_access("cid", "csecret");
        assert_eq!(req["method"], "requestAccess");
        assert_eq!(req["id"], REQUEST_ACCESS_ID);
    }

    #[test]
    fn test_authorize() {
        let req = authorize("cid", "csecret", "lic", 100);
        assert_eq!(req["method"], "authorize");
        assert_eq!(req["id"], AUTHORIZE_ID);
        assert_eq!(req["params"]["license"], "lic");
        assert_eq!(req["params"]["debit"], 100);
    }

    #[test]
    fn test_query_headsets() {
        let req = query_headsets();
        assert_eq!(req["method"], "queryHeadsets");
        assert_eq!(req["id"], QUERY_HEADSET_ID);
        assert!(req["params"].is_object());
    }

    #[test]
    fn test_connect_headset() {
        let req = connect_headset("EPOCX-12345");
        assert_eq!(req["method"], "controlDevice");
        assert_eq!(req["params"]["command"], "connect");
        assert_eq!(req["params"]["headset"], "EPOCX-12345");
    }

    #[test]
    fn test_disconnect_headset() {
        let req = disconnect_headset("EPOCX-12345");
        assert_eq!(req["method"], "controlDevice");
        assert_eq!(req["params"]["command"], "disconnect");
        assert_eq!(req["params"]["headset"], "EPOCX-12345");
    }

    #[test]
    fn test_refresh_headset_list() {
        let req = refresh_headset_list();
        assert_eq!(req["method"], "controlDevice");
        assert_eq!(req["params"]["command"], "refresh");
    }

    #[test]
    fn test_create_session() {
        let req = create_session("tok123", "hs-001");
        assert_eq!(req["method"], "createSession");
        assert_eq!(req["params"]["cortexToken"], "tok123");
        assert_eq!(req["params"]["headset"], "hs-001");
        assert_eq!(req["params"]["status"], "active");
    }

    #[test]
    fn test_close_session() {
        let req = close_session("tok123", "ses-001");
        assert_eq!(req["method"], "updateSession");
        assert_eq!(req["params"]["session"], "ses-001");
        assert_eq!(req["params"]["status"], "close");
    }

    #[test]
    fn test_subscribe() {
        let req = subscribe("tok", "ses", &["eeg", "mot"]);
        assert_eq!(req["method"], "subscribe");
        assert_eq!(req["params"]["streams"][0], "eeg");
        assert_eq!(req["params"]["streams"][1], "mot");
    }

    #[test]
    fn test_unsubscribe() {
        let req = unsubscribe("tok", "ses", &["eeg"]);
        assert_eq!(req["method"], "unsubscribe");
        assert_eq!(req["params"]["streams"][0], "eeg");
    }

    #[test]
    fn test_create_record() {
        let req = create_record("tok", "ses", "My Record", "desc");
        assert_eq!(req["method"], "createRecord");
        assert_eq!(req["params"]["title"], "My Record");
        assert_eq!(req["params"]["description"], "desc");
    }

    #[test]
    fn test_stop_record() {
        let req = stop_record("tok", "ses");
        assert_eq!(req["method"], "stopRecord");
        assert_eq!(req["params"]["session"], "ses");
    }

    #[test]
    fn test_export_record_csv() {
        let req = export_record("tok", "/tmp", "CSV", &["EEG"], &["rec-1"], "V2");
        assert_eq!(req["method"], "exportRecord");
        assert_eq!(req["params"]["format"], "CSV");
        assert_eq!(req["params"]["version"], "V2");
        assert_eq!(req["params"]["folder"], "/tmp");
    }

    #[test]
    fn test_export_record_edf_no_version() {
        let req = export_record("tok", "/tmp", "EDF", &["EEG"], &["rec-1"], "V2");
        assert_eq!(req["method"], "exportRecord");
        assert_eq!(req["params"]["format"], "EDF");
        // EDF should not include "version"
        assert!(req["params"].get("version").is_none());
    }

    #[test]
    fn test_inject_marker() {
        let req = inject_marker("tok", "ses", 12345.0, "val", "lbl");
        assert_eq!(req["method"], "injectMarker");
        assert_eq!(req["params"]["value"], "val");
        assert_eq!(req["params"]["label"], "lbl");
        assert_eq!(req["params"]["time"], 12345.0);
    }

    #[test]
    fn test_update_marker() {
        let req = update_marker("tok", "ses", "mk-1", 99.0);
        assert_eq!(req["method"], "updateMarker");
        assert_eq!(req["params"]["markerId"], "mk-1");
        assert_eq!(req["params"]["time"], 99.0);
    }

    #[test]
    fn test_query_profile() {
        let req = query_profile("tok");
        assert_eq!(req["method"], "queryProfile");
        assert_eq!(req["params"]["cortexToken"], "tok");
    }

    #[test]
    fn test_setup_profile() {
        let req = setup_profile("tok", "hs", "myprof", "load");
        assert_eq!(req["method"], "setupProfile");
        assert_eq!(req["params"]["profile"], "myprof");
        assert_eq!(req["params"]["status"], "load");
    }

    #[test]
    fn test_train_request() {
        let req = train_request("tok", "ses", "mentalCommand", "push", "start");
        assert_eq!(req["method"], "training");
        assert_eq!(req["params"]["detection"], "mentalCommand");
        assert_eq!(req["params"]["action"], "push");
        assert_eq!(req["params"]["status"], "start");
    }

    #[test]
    fn test_get_mental_command_active_action() {
        let req = get_mental_command_active_action("tok", "prof");
        assert_eq!(req["method"], "mentalCommandActiveAction");
        assert_eq!(req["params"]["status"], "get");
    }

    #[test]
    fn test_set_mental_command_active_action() {
        let req = set_mental_command_active_action("tok", "ses", &["push", "pull"]);
        assert_eq!(req["method"], "mentalCommandActiveAction");
        assert_eq!(req["params"]["status"], "set");
        assert_eq!(req["params"]["actions"][0], "push");
    }

    #[test]
    fn test_get_mental_command_sensitivity() {
        let req = get_mental_command_sensitivity("tok", "prof");
        assert_eq!(req["method"], "mentalCommandActionSensitivity");
        assert_eq!(req["params"]["status"], "get");
    }

    #[test]
    fn test_set_mental_command_sensitivity() {
        let req = set_mental_command_sensitivity("tok", "prof", "ses", &[7, 8, 5, 5]);
        assert_eq!(req["method"], "mentalCommandActionSensitivity");
        assert_eq!(req["params"]["status"], "set");
        assert_eq!(req["params"]["values"][0], 7);
        assert_eq!(req["params"]["values"][3], 5);
    }

    #[test]
    fn test_get_mental_command_brain_map() {
        let req = get_mental_command_brain_map("tok", "prof", "ses");
        assert_eq!(req["method"], "mentalCommandBrainMap");
    }

    #[test]
    fn test_get_mental_command_training_threshold() {
        let req = get_mental_command_training_threshold("tok", "ses");
        assert_eq!(req["method"], "mentalCommandTrainingThreshold");
    }

    #[test]
    fn test_query_records() {
        let req = query_records("tok", json!({"orderBy": [{"startDatetime": "DESC"}]}));
        assert_eq!(req["method"], "queryRecords");
        assert_eq!(req["params"]["cortexToken"], "tok");
        assert!(req["params"]["orderBy"].is_array());
    }

    #[test]
    fn test_request_download_records() {
        let req = request_download_records("tok", &["rec-1", "rec-2"]);
        assert_eq!(req["method"], "requestToDownloadRecordData");
        assert_eq!(req["params"]["recordIds"][0], "rec-1");
        assert_eq!(req["params"]["recordIds"][1], "rec-2");
    }

    #[test]
    fn test_sync_with_headset_clock() {
        let req = sync_with_headset_clock("hs-001");
        assert_eq!(req["method"], "syncWithHeadsetClock");
        assert_eq!(req["params"]["headset"], "hs-001");
        assert!(req["params"]["systemTime"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_get_cortex_info() {
        let req = get_cortex_info();
        assert_eq!(req["method"], "getCortexInfo");
        assert_eq!(req["id"], GET_CORTEX_INFO_ID);
    }

    #[test]
    fn test_constants() {
        assert_eq!(CORTEX_WS_URL, "wss://localhost:6868");
        assert_eq!(EEG_FREQUENCY, 128.0);
        assert_eq!(STREAM_EEG, "eeg");
        assert_eq!(STREAM_COM, "com");
        assert_eq!(ERR_PROFILE_ACCESS_DENIED, -32046);
    }
}
