use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use shoal_proto::{
    AttachFailureData, AttachFailureReason, AttachResult, CaptureResult, CoreMismatchData,
    DaemonStatusResult, Empty, Frame, HelloResult, Notification, PaneResult, ProtocolMismatchData,
    RawNotification, RpcError, SessionResult, SizeResult, TabCreateResult, WorkspaceState,
    decode_exact, error_code,
};
use std::collections::BTreeSet;
use std::fmt::Debug;

use serde_json::json;
use shoal_proto::{Call, Message, Request, RequestError, RequestId, Response};

#[test]
fn hello_matches_spec_example() {
    let value = json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{
        "protocol":1,"client":"gui","version":"0.1.0","core":"core-id",
        "env":{"WAYLAND_DISPLAY":"wayland-1"}
    }});
    let Message::Request(request) = serde_json::from_value::<Message>(value.clone()).unwrap()
    else {
        panic!("request")
    };
    assert!(
        matches!(request.call().unwrap(), Call::Hello { core: Some(core), .. } if core == "core-id")
    );
    assert_eq!(serde_json::to_value(request).unwrap(), value);
}

#[test]
fn success_null_is_a_result_and_is_not_an_error() {
    let response: Response<serde_json::Value> =
        serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":null})).unwrap();
    assert_eq!(
        response,
        Response::Success {
            id: RequestId::from(1_u64),
            result: serde_json::Value::Null
        }
    );
}

fn round_trip_response<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: Value) {
    let response: Response<T> = serde_json::from_value(value.clone()).unwrap();
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire, value);
    assert_eq!(
        serde_json::from_value::<Response<T>>(wire).unwrap(),
        response
    );
}

#[test]
fn every_m1_request_and_response_matches_reviewable_wire_fixtures() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/m1-methods.json")).unwrap();
    let names: BTreeSet<_> = fixtures
        .iter()
        .map(|fixture| fixture["method"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), fixtures.len());
    assert_eq!(names, Call::METHOD_NAMES.iter().copied().collect());
    for fixture in &fixtures {
        let method = fixture["method"].as_str().unwrap();
        let request_value =
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":fixture["params"]});
        let request: Request = serde_json::from_value(request_value.clone()).unwrap();
        let call = request.call().unwrap();
        let constructed = Request::from_call(RequestId::from(1_u64), &call).unwrap();
        assert_eq!(constructed, request, "{method}");
        assert_eq!(
            serde_json::to_value(constructed).unwrap(),
            request_value,
            "{method}"
        );
        let frame = Frame::Json(request_value.as_object().unwrap().clone());
        let wire = frame.encode().unwrap();
        let Frame::Json(object) = decode_exact(&wire).unwrap() else {
            panic!("JSON")
        };
        assert_eq!(
            Message::from_object(object).unwrap(),
            Message::Request(request)
        );

        let response = json!({"jsonrpc":"2.0","id":1,"result":fixture["result"]});
        match method {
            "hello" => round_trip_response::<HelloResult>(response),
            "workspace.get" => round_trip_response::<WorkspaceState>(response),
            "session.open_smart" | "session.create" => {
                round_trip_response::<SessionResult>(response)
            }
            "tab.create" => round_trip_response::<TabCreateResult>(response),
            "pane.split" => round_trip_response::<PaneResult>(response),
            "pane.attach" => round_trip_response::<AttachResult>(response),
            "pane.set_size" => round_trip_response::<SizeResult>(response),
            "pane.capture" => round_trip_response::<CaptureResult>(response),
            "daemon.status" => round_trip_response::<DaemonStatusResult>(response),
            _ => round_trip_response::<Empty>(response),
        }
    }
}

#[test]
fn every_m1_notification_matches_literal_envelope() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/m1-notifications.json")).unwrap();
    for value in fixtures {
        let raw: RawNotification = serde_json::from_value(value.clone()).unwrap();
        let typed = raw.notification().unwrap();
        assert_eq!(RawNotification::from_notification(&typed).unwrap(), raw);
        assert_eq!(serde_json::to_value(&raw).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<Message>(value).unwrap(),
            Message::Notification(raw)
        );
    }
    assert!(
        serde_json::from_value::<Notification>(
            json!({"method":"pane.event","params":{"pane":"p3","event":{}}})
        )
        .is_err()
    );
}

#[test]
fn optional_fields_cli_hello_empty_params_and_null_rename_follow_spec() {
    for (method, params) in [
        (
            "hello",
            json!({"protocol":1,"client":"cli","version":"0.1.0","env":{}}),
        ),
        ("session.open_smart", json!({})),
        ("session.create", json!({})),
        ("tab.create", json!({"session":"s1"})),
        ("pane.split", json!({"pane":"p3","direction":"down"})),
        ("tab.rename", json!({"tab":"t2","name":"build"})),
        (
            "pane.capture",
            json!({"pane":"p3","format":"text","scrollback":false}),
        ),
    ] {
        let request: Request = serde_json::from_value(
            json!({"jsonrpc":"2.0","id":"scripting","method":method,"params":params}),
        )
        .unwrap();
        let typed = request.call().unwrap();
        assert_eq!(
            Request::from_call(RequestId::from("scripting"), &typed).unwrap(),
            request
        );
    }
    for method in [
        "workspace.get",
        "workspace.subscribe",
        "daemon.status",
        "daemon.persist",
        "daemon.shutdown",
    ] {
        let request: Request =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":method})).unwrap();
        assert_eq!(request.params, json!({}));
        assert_eq!(
            serde_json::to_value(
                Request::from_call(request.id.clone(), &request.call().unwrap()).unwrap()
            )
            .unwrap(),
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":{}})
        );
    }
    let status = json!({"jsonrpc":"2.0","id":1,"result":{"version":"0.1.0","pid":4312,"uptime_s":0,"sessions":0,"panes":0,"last_persist_at":null}});
    round_trip_response::<DaemonStatusResult>(status);
}

#[test]
fn malformed_envelopes_are_rejected_instead_of_becoming_notifications() {
    for value in [
        json!({}),
        json!([]),
        json!(null),
        json!({"id":1,"method":"hello","params":{}}),
        json!({"jsonrpc":"1.0","id":1,"method":"hello","params":{}}),
        json!({"jsonrpc":"2.0","id":1,"method":null,"params":{}}),
        json!({"jsonrpc":"2.0","id":null,"method":"hello","params":{}}),
        json!({"jsonrpc":"2.0","id":true,"method":"hello","params":{}}),
        json!({"jsonrpc":"2.0","id":1.5,"method":"hello","params":{}}),
        json!({"jsonrpc":"2.0","id":[],"method":"hello","params":{}}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1005,"message":"Error"}}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"method":"hello"}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"params":{}}),
        json!({"jsonrpc":"2.0","result":{}}),
        json!({"jsonrpc":"2.0","id":null,"result":{}}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"error":null}),
        json!({"jsonrpc":"2.0","id":1,"error":[1005,"Error",null]}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":"1005","message":"Error"}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":1005}}),
    ] {
        assert!(
            serde_json::from_value::<Message>(value.clone()).is_err(),
            "accepted {value}"
        );
    }
}

#[test]
fn invalid_method_parameters_have_bad_params_code() {
    for (method, params) in [
        ("pane.close", json!({"pane":"s3"})),
        ("pane.attach", json!({"pane":3})),
        (
            "pane.resize",
            json!({"pane":"p3","direction":"left","cells":-1}),
        ),
        (
            "pane.resize",
            json!({"pane":"p3","direction":"left","cells":1.5}),
        ),
        ("pane.split", json!({"pane":"p3","direction":"up"})),
        ("pane.set_size", json!({"pane":"p3","cols":65536,"rows":40})),
        ("tab.move", json!({"tab":"t2","index":-1})),
        ("split.set_ratio", json!({"split":"x4","ratio":1e100})),
        ("tab.rename", json!({"tab":"t2"})),
        (
            "hello",
            json!({"protocol":1,"client":"agent","version":"0.1.0","env":{}}),
        ),
        (
            "pane.capture",
            json!({"pane":"p3","format":"json","scrollback":false}),
        ),
        ("workspace.get", json!({"extra":true})),
        ("workspace.get", json!([])),
        ("workspace.get", json!(null)),
    ] {
        let request = Request {
            id: RequestId::from(1_u64),
            method: method.into(),
            params,
        };
        let error = request.call().expect_err(method);
        assert!(matches!(error, RequestError::BadParams(_)));
        assert_eq!(error.code(), 1004);
    }
    for params in [json!({}), json!(null), json!([1]), json!(false)] {
        let request = Request {
            id: RequestId::from(1_u64),
            method: "future.method".into(),
            params,
        };
        assert_eq!(request.call().unwrap_err().code(), -32601);
    }
}

#[test]
fn string_signed_and_full_width_unsigned_ids_round_trip() {
    for id in [
        json!(""),
        json!("request-3"),
        json!(0),
        json!(-1),
        json!(i64::MIN),
        json!(u64::MAX),
    ] {
        let wire = json!({"jsonrpc":"2.0","id":id,"method":"daemon.status","params":{}});
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        round_trip_response::<Empty>(json!({"jsonrpc":"2.0","id":id,"result":{}}));
    }
}

#[test]
fn every_error_code_and_specified_data_shape_round_trips() {
    let codes = [
        (error_code::INVALID_REQUEST, -32600),
        (error_code::UNKNOWN_METHOD, -32601),
        (error_code::NOT_FOUND, 1001),
        (error_code::INVALID_STATE, 1002),
        (error_code::VERSION_MISMATCH, 1003),
        (error_code::BAD_PARAMS, 1004),
        (error_code::INTERNAL, 1005),
    ];
    for (actual, expected) in codes {
        assert_eq!(actual, expected);
        let value = json!({"jsonrpc":"2.0","id":null,"error":{"code":expected,"message":"Error"}});
        round_trip_response::<Empty>(value);
    }
    let data = [
        (
            json!({"daemon_protocol":1,"pid":4312}),
            serde_json::to_value(ProtocolMismatchData {
                daemon_protocol: 1,
                pid: 4312,
            })
            .unwrap(),
            1003,
        ),
        (
            json!({"daemon_core":"other-core"}),
            serde_json::to_value(CoreMismatchData {
                daemon_core: "other-core".into(),
            })
            .unwrap(),
            1003,
        ),
        (
            json!({"reason":"too_large"}),
            serde_json::to_value(AttachFailureData {
                reason: AttachFailureReason::TooLarge,
            })
            .unwrap(),
            1002,
        ),
        (
            json!({"reason":"mid_sequence"}),
            serde_json::to_value(AttachFailureData {
                reason: AttachFailureReason::MidSequence,
            })
            .unwrap(),
            1002,
        ),
    ];
    for (expected, actual, code) in data {
        assert_eq!(actual, expected);
        let value =
            json!({"jsonrpc":"2.0","id":1,"error":{"code":code,"message":"Error","data":expected}});
        round_trip_response::<Empty>(value);
    }
    let error = RpcError {
        code: 1001,
        message: "Not found".into(),
        data: None,
    };
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({"code":1001,"message":"Not found"})
    );
}

#[test]
fn nonfinite_ratio_cannot_be_encoded_as_a_null_parameter() {
    for ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let call = Call::SplitSetRatio {
            split: shoal_model::SplitId::new(4),
            ratio,
        };
        assert!(Request::from_call(RequestId::from(1_u64), &call).is_err());
    }
}
