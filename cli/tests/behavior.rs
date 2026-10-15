use bulbul_birding::{
    Result, ToolError,
    account::{CredentialStore, Environment, Session, normalize_api_url, read_json_response},
    client::{BulbulClient, TOOLS},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

fn test_directory() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(root).unwrap();
    // Keep artifacts under the repository's task directory for trash cleanup.
    tempfile::Builder::new()
        .prefix("behavior-")
        .tempdir_in(root)
        .unwrap()
        .keep()
}

#[derive(Default)]
struct MemoryStore {
    sessions: Mutex<HashMap<String, String>>,
    selected: Mutex<Option<String>>,
    fail_save: bool,
    unavailable: bool,
}

impl CredentialStore for MemoryStore {
    fn api_url(&self) -> Result<Option<String>> {
        Ok(self.selected.lock().unwrap().clone())
    }
    fn ensure_available(&self, _: &str) -> Result<()> {
        if self.unavailable {
            Err(ToolError::new("系统凭据库不可用"))
        } else {
            Ok(())
        }
    }
    fn read(&self, url: &str) -> Result<Option<Session>> {
        if self.unavailable {
            return Err(ToolError::coded(
                "credential_access_denied",
                "当前进程无法访问系统凭据库",
            ));
        }
        Ok(self.sessions.lock().unwrap().get(url).map(|token| Session {
            session_token: token.clone(),
            expires_at: None,
        }))
    }
    fn save(&self, url: &str, session: &Session) -> Result<()> {
        if self.fail_save {
            return Err(ToolError::new("无法保存登录"));
        }
        self.sessions
            .lock()
            .unwrap()
            .insert(url.to_owned(), session.session_token.clone());
        *self.selected.lock().unwrap() = Some(url.to_owned());
        Ok(())
    }
    fn clear(&self, url: &str) -> Result<()> {
        self.sessions.lock().unwrap().remove(url);
        Ok(())
    }
}

fn client(server: &MockServer, store: Arc<MemoryStore>) -> BulbulClient {
    BulbulClient::new(store, Environment::default(), Some(&server.uri())).unwrap()
}

fn with_token(server: &MockServer) -> BulbulClient {
    BulbulClient::new(
        Arc::new(MemoryStore::default()),
        Environment {
            session_token: Some("private-test-token".into()),
            ..Default::default()
        },
        Some(&server.uri()),
    )
    .unwrap()
}

async fn login_start(server: &MockServer) {
    Mock::given(method("POST")).and(path("/api/v1/auth/device/start")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code":"d".repeat(43),"user_code":"ABCD-2345","verification_uri":format!("{}/auth/device?code=ABCD-2345",server.uri()),"expires_in":600,"interval":3}))).expect(1).mount(server).await;
}

#[test]
fn origins_are_normalized_and_unsafe_urls_rejected() {
    for url in [
        "http://remote.example",
        "https://user:secret@example.com",
        "https://example.com?token=secret",
        "https://example.com/admin",
        "https://example.com#fragment",
        "https://example.com:invalid",
    ] {
        let error = normalize_api_url(url).unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
    assert_eq!(
        normalize_api_url("https://EXAMPLE.com/api/v1/").unwrap(),
        "https://example.com/api/v1"
    );
    assert_eq!(
        normalize_api_url("http://[::1]:8000").unwrap(),
        "http://[::1]:8000/api/v1"
    );
}

#[tokio::test]
async fn schemas_reject_invalid_arguments_before_http() {
    let server = MockServer::start().await;
    let client = with_token(&server);
    for (tool, arguments) in [
        ("not-a-tool", json!({})),
        ("location_search", json!({"query":"   "})),
        ("species_wiki", json!({"species_code":"private-input"})),
        ("hotspots_nearby", json!({"lat":100,"lon":120})),
        (
            "species_predict",
            json!({"lat":24,"lon":120,"target_date":"tomorrow"}),
        ),
        (
            "species_predict",
            json!({"lat":24,"lon":120,"target_date":"2026-02-30"}),
        ),
        ("species_list_detail", json!({"list_id":"../another-user"})),
        ("species_list_detail", json!({"list_id":".."})),
        (
            "species_list_create",
            json!({"name":"x","taxon_ids":[1],"confirm":"true"}),
        ),
        (
            "species_detail",
            json!({"taxon_id":7,"prediction_taxon_ids":["private-input"]}),
        ),
        ("species_lists", json!([])),
    ] {
        let error = client.call(tool, arguments).await.unwrap_err().to_string();
        assert!(!error.contains("private-input"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn every_write_requires_preview_and_confirm_is_not_sent_to_api() {
    let server = MockServer::start().await;
    let client = with_token(&server);
    for (tool, mut arguments) in [
        ("quiz_route_book_create", json!({"snapshot_id":"saved"})),
        (
            "quiz_custom_batch_create",
            json!({"species_list_id":"list"}),
        ),
        (
            "species_list_create",
            json!({"name":"预习","taxon_ids":[1]}),
        ),
        (
            "species_list_from_saved_query",
            json!({"name":"预习","saved_query_id":"saved"}),
        ),
        (
            "species_list_from_taxonomy",
            json!({"name":"预习","rank":"family","values":["Pycnonotidae"]}),
        ),
        (
            "species_list_replace_items",
            json!({"list_id":"list","taxon_ids":[1,2]}),
        ),
    ] {
        let before = server.received_requests().await.unwrap().len();
        let preview = client.call(tool, arguments.clone()).await.unwrap();
        assert_eq!(preview["status"], "confirmation_required");
        assert_eq!(server.received_requests().await.unwrap().len(), before);
        let body = preview["body"].clone();
        Mock::given(method(preview["method"].as_str().unwrap()))
            .and(path(format!(
                "/api/v1{}",
                preview["path"].as_str().unwrap()
            )))
            .and(header("authorization", "Bearer private-test-token"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"list_id":"created"})))
            .expect(1)
            .mount(&server)
            .await;
        arguments["confirm"] = json!(true);
        assert_eq!(
            client.call(tool, arguments).await.unwrap()["list_id"],
            "created"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), before + 1);
    }
}

#[tokio::test]
async fn reads_use_taxon_id_defaults_and_encode_query_parameters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/species/42/wiki"))
        .and(header("authorization", "Bearer private-test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"taxon_id":42})))
        .expect(1)
        .mount(&server)
        .await;
    let client = with_token(&server);
    assert_eq!(
        client
            .call("species_wiki", json!({"taxon_id":42}))
            .await
            .unwrap()["taxon_id"],
        42
    );
    Mock::given(method("GET"))
        .and(path("/api/v1/species/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[]})))
        .mount(&server)
        .await;
    client
        .call("species_catalog", json!({"q":"白头鹎 & +","limit":5}))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[1]
            .url
            .query_pairs()
            .any(|(key, value)| key == "q" && value == "白头鹎 & +")
    );
}

#[tokio::test]
async fn results_report_truncation_for_arrays_and_envelopes() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/species/predict"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"taxon_id":1},{"taxon_id":2},{"taxon_id":3}])),
        )
        .mount(&server)
        .await;
    let client = with_token(&server);
    let result = client
        .call(
            "species_predict",
            json!({"lat":24,"lon":120,"target_date":"2026-10-13","limit":2}),
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"items":[{"taxon_id":1},{"taxon_id":2}],"total":3,"truncated":true})
    );
    Mock::given(path("/api/v1/me/saved-queries"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"snapshots":[{"id":"one"},{"id":"two"}],"metadata":"preserved"}),
            ),
        )
        .mount(&server)
        .await;
    let result = client
        .call("saved_queries", json!({"limit":1}))
        .await
        .unwrap();
    assert_eq!(result["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(result["total"], 2);
    assert_eq!(result["metadata"], "preserved");
}

#[tokio::test]
async fn failed_requests_are_not_retried_redirected_or_disclosed() {
    for status in [401, 403, 409, 429, 500, 302] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/me/species-lists"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Location", "https://other.example/")
                    .set_body_json(json!({"detail":"private-test-token private-map-key"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = with_token(&server)
            .call("species_list_create", json!({"name":"x","confirm":true}))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-test-token"));
        assert!(!error.contains("private-map-key"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn login_is_shared_with_running_clients_but_not_other_origins() {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryStore::default());
    let running = client(&server, store.clone());
    assert_eq!(
        running.account.status().await.unwrap()["status"],
        "signed_out"
    );
    login_start(&server).await;
    Mock::given(path("/api/v1/auth/device/poll")).and(body_json(json!({"device_code":"d".repeat(43)}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"authorized","session_token":"saved-private-token","expires_at":"2026-11-12T00:00:00Z","user":{"email":"birder@example.com"}}))).mount(&server).await;
    let login = client(&server, store.clone());
    let info = login.account.begin_login(false).await.unwrap();
    assert!(!info.to_string().contains("device_code"));
    let result = login.account.complete_login().await.unwrap();
    assert_eq!(result["status"], "logged_in");
    assert!(!result.to_string().contains("saved-private-token"));
    assert!(
        !store
            .api_url()
            .unwrap()
            .unwrap()
            .contains("saved-private-token")
    );
    assert_eq!(
        running.account.token().unwrap().unwrap(),
        "saved-private-token"
    );
    let other = MockServer::start().await;
    assert!(client(&other, store).account.token().unwrap().is_none());
}

#[tokio::test]
async fn pending_denied_and_expired_authorizations_do_not_save_credentials() {
    for status in ["pending", "denied", "expired"] {
        let server = MockServer::start().await;
        let store = Arc::new(MemoryStore::default());
        login_start(&server).await;
        Mock::given(path("/api/v1/auth/device/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":status})))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server, store.clone());
        client.account.begin_login(false).await.unwrap();
        let result = client.account.complete_login().await;
        if status == "expired" {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap()["status"], status);
        }
        if status == "pending" {
            assert_eq!(
                client.account.complete_login().await.unwrap()["status"],
                "pending"
            );
        } else {
            assert!(client.account.complete_login().await.is_err());
        }
        assert!(store.sessions.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn a_failed_save_revokes_the_new_session() {
    let server = MockServer::start().await;
    login_start(&server).await;
    Mock::given(path("/api/v1/auth/device/poll"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"status":"authorized","session_token":"unsaved-private-token"}),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/auth/logout"))
        .and(header("authorization", "Bearer unsaved-private-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let store = Arc::new(MemoryStore {
        fail_save: true,
        ..Default::default()
    });
    let client = client(&server, store.clone());
    client.account.begin_login(false).await.unwrap();
    assert!(client.account.complete_login().await.is_err());
    assert!(store.sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unavailable_keyring_and_environment_token_prevent_browser_login() {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryStore {
        unavailable: true,
        ..Default::default()
    });
    assert!(
        client(&server, store)
            .account
            .begin_login(false)
            .await
            .is_err()
    );
    assert!(
        with_token(&server)
            .account
            .begin_login(false)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_login_urls_are_rejected() {
    for url in [
        "http://remote.example/auth/device?code=ABCD-2345",
        "http://localhost/auth/device?code=wrong",
        "https://user:secret@localhost/auth/device",
        "https://example.com/other?code=ABCD-2345",
        "https://example.com/auth/device?code=ABCD-2345&device_code=secret",
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/device/start")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code":"d".repeat(43),"user_code":"ABCD-2345","verification_uri":url,"expires_in":600,"interval":3}))).mount(&server).await;
        let error = client(&server, Arc::new(MemoryStore::default()))
            .account
            .begin_login(false)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"));
        assert!(!error.contains("remote.example"));
    }
}

#[tokio::test]
async fn login_allows_a_separate_web_app_origin() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/auth/device/start"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code":"d".repeat(43),"user_code":"ABCD-2345",
            "verification_uri":"http://localhost:5173/auth/device?code=ABCD-2345",
            "expires_in":600,"interval":3
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server, Arc::new(MemoryStore::default()));
    let result = client.account.begin_login(false).await.unwrap();
    assert_eq!(
        result["verification_url"],
        "http://localhost:5173/auth/device?code=ABCD-2345"
    );
}

#[tokio::test]
async fn logout_preserves_credentials_on_failure_and_only_clears_current_site() {
    for status in [503, 204] {
        let server = MockServer::start().await;
        let store = Arc::new(MemoryStore::default());
        let api = normalize_api_url(&server.uri()).unwrap();
        store
            .save(
                &api,
                &Session {
                    session_token: "saved-private-token".into(),
                    expires_at: None,
                },
            )
            .unwrap();
        store
            .save(
                "https://other.example/api/v1",
                &Session {
                    session_token: "other-token".into(),
                    expires_at: None,
                },
            )
            .unwrap();
        Mock::given(path("/api/v1/auth/logout"))
            .and(header("authorization", "Bearer saved-private-token"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let result = client(&server, store.clone()).account.logout().await;
        assert_eq!(result.is_ok(), status == 204);
        assert_eq!(store.read(&api).unwrap().is_none(), status == 204);
        assert!(
            store
                .read("https://other.example/api/v1")
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn location_search_uses_saved_login_and_needs_no_map_key() {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryStore::default());
    store
        .save(
            &normalize_api_url(&server.uri()).unwrap(),
            &Session {
                session_token: "saved-private-token".into(),
                expires_at: None,
            },
        )
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/locations/search"))
        .and(query_param("query", "翠湖"))
        .and(query_param("limit", "5"))
        .and(header("authorization", "Bearer saved-private-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "source":"tianditu", "query":"翠湖",
            "candidates":[{"name":"翠湖", "lon":102.704, "lat":25.048, "address":"昆明"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let result = client(&server, store)
        .call("location_search", json!({"query":"翠湖"}))
        .await
        .unwrap();
    assert_eq!(result["candidates"][0]["name"], "翠湖");
    assert_eq!(result["candidates"][0]["lon"], 102.704);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let keys = requests[0]
        .url
        .query_pairs()
        .map(|(key, _)| key.into_owned())
        .collect::<Vec<_>>();
    assert_eq!(keys.len(), 2);
    assert!(!keys.iter().any(|key| key == "tk" || key == "postStr"));
}

#[tokio::test]
async fn location_search_requires_login_before_sending_a_request() {
    let server = MockServer::start().await;
    let error = client(&server, Arc::new(MemoryStore::default()))
        .call("location_search", json!({"query":"翠湖"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bulbul login"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn cli_reads_bom_files_and_never_echoes_bad_json_or_command_arguments() {
    let directory = test_directory();
    let file = directory.join("arguments.json");
    std::fs::write(&file, "\u{feff}{\"name\":\"预习\",\"taxon_ids\":[1]}").unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_bulbul"))
            .current_dir(&directory)
            .env_remove("BULBUL_API_URL")
            .env_remove("BULBUL_SESSION_TOKEN")
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&[
        "call",
        "species_list_create",
        "--arguments-file",
        file.to_str().unwrap(),
    ]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"],
        "confirmation_required"
    );
    for args in [
        &["call", "species_wiki", "--arguments", "private-secret"][..],
        &["--private-secret"][..],
        &["mcp"][..],
        &["call", "species_wiki", "--confirm"][..],
    ] {
        let output = run(args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-secret"));
    }
    let output = run(&["list"]);
    assert!(output.status.success());
    let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(tools.as_array().unwrap().len(), TOOLS.len());
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["input_schema"]["type"] == "object")
    );
    let output = run(&["list", "species_wiki"]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()[0]["name"],
        "species_wiki"
    );
    let output = run(&["list", "species_wiki", "identify_photo"]);
    assert!(output.status.success());
    let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(tools[0]["name"], "species_wiki");
    assert_eq!(tools[1]["name"], "identify_photo");
    let output = run(&["list", "species_wiki", "private-invalid-name"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-invalid-name"));
}

#[tokio::test]
async fn query_arrays_send_repeated_ids_while_json_body_arrays_stay_arrays() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/hotspots/target-destinations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[]})))
        .expect(3)
        .mount(&server)
        .await;
    let client = with_token(&server);
    for ids in [vec![42, 73, 42], vec![42], vec![]] {
        client
            .call(
                "hotspots_target_destinations",
                json!({"target_date":"2026-10-16", "taxon_id":ids, "species_list_id":null}),
            )
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let request = requests.last().unwrap();
        let actual: Vec<String> = request
            .url
            .query_pairs()
            .filter(|(key, _)| key == "taxon_id")
            .map(|(_, v)| v.into_owned())
            .collect();
        assert_eq!(actual, ids.iter().map(i32::to_string).collect::<Vec<_>>());
        assert!(
            !request
                .url
                .query_pairs()
                .any(|(key, _)| key == "species_list_id")
        );
    }
    Mock::given(method("POST"))
        .and(path("/api/v1/species/42/detail"))
        .and(body_json(
            json!({"prediction_taxon_ids":[42,73], "compare_limit":3}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    client
        .call(
            "species_detail",
            json!({"taxon_id":42, "prediction_taxon_ids":[42,73]}),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn validation_errors_expose_only_safe_fields_and_types() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/hotspots/target-destinations"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"detail":[
            {"loc":["query","taxon_id",0], "type":"int_parsing", "msg":"private-token", "input":"private-input", "ctx":{"value":"private-secret"}},
            {"loc":["body","private-field"], "type":"private-type", "msg":"private-message"}
        ]}))).expect(1).mount(&server).await;
    let error = with_token(&server)
        .call(
            "hotspots_target_destinations",
            json!({"target_date":"2026-10-16","taxon_id":[42]}),
        )
        .await
        .unwrap_err()
        .as_json();
    assert_eq!(error["code"], "invalid_arguments");
    assert_eq!(error["details"]["http_status"], 422);
    assert_eq!(
        error["details"]["validation"][0],
        json!({"field":["query","taxon_id",0],"type":"integer"})
    );
    assert_eq!(
        error["details"]["validation"][1],
        json!({"field":["body","field"],"type":"validation_error"})
    );
    assert!(!error.to_string().contains("private-"));
}

#[tokio::test]
async fn validation_error_bodies_are_bounded_and_invalid_json_is_safe() {
    for body in ["private-body".to_owned(), "x".repeat(70 * 1024)] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/locations/search"))
            .respond_with(ResponseTemplate::new(422).set_body_string(body))
            .mount(&server)
            .await;
        let error = with_token(&server)
            .call("location_search", json!({"query":"翠湖"}))
            .await
            .unwrap_err()
            .as_json();
        assert_eq!(error["details"]["validation"], json!([]));
        assert!(!error.to_string().contains("private-body"));
    }
}

#[tokio::test]
async fn network_and_timeout_errors_are_distinct_and_never_echo_request_urls() {
    let server = MockServer::start().await;
    Mock::given(path("/private-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(200))
                .set_body_json(json!({})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = reqwest::Client::new();
    let error = read_json_response(
        http.get(format!("{}/private-token", server.uri()))
            .timeout(std::time::Duration::from_millis(40)),
    )
    .await
    .unwrap_err()
    .as_json();
    assert_eq!(error["code"], "request_timeout");
    assert!(!error.to_string().contains("private-token"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let error = read_json_response(http.get(format!("http://127.0.0.1:{port}/private-token")))
        .await
        .unwrap_err()
        .as_json();
    assert_eq!(error["code"], "network_unavailable");
    assert!(!error.to_string().contains("private-token"));
}

#[tokio::test]
async fn doctor_is_local_keeps_sessions_and_distinguishes_missing_from_denied_access() {
    let server = MockServer::start().await;
    let store = Arc::new(MemoryStore::default());
    let client = client(&server, store.clone());
    assert_eq!(client.account.doctor()["session_present"], false);
    store
        .save(
            &client.account.api_url,
            &Session {
                session_token: "private-saved-token".into(),
                expires_at: None,
            },
        )
        .unwrap();
    let report = client.account.doctor();
    assert_eq!(report["session_present"], true);
    assert_eq!(report["server_session"], "not_checked");
    assert!(!report.to_string().contains("private-saved-token"));
    assert!(store.read(&client.account.api_url).unwrap().is_some());
    let denied = BulbulClient::new(
        Arc::new(MemoryStore {
            unavailable: true,
            ..Default::default()
        }),
        Environment::default(),
        Some(&server.uri()),
    )
    .unwrap()
    .account
    .doctor();
    assert_eq!(denied["credential_store"], "access_denied");
    assert!(denied["session_present"].is_null());
    let env = with_token(&server).account.doctor();
    assert_eq!(env["session_source"], "environment");
    assert_eq!(env["credential_store"], "not_checked");
    assert!(!env.to_string().contains("private-test-token"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn uploads_local_files_with_auth_and_cloud_calibration_without_disclosing_paths() {
    let server = MockServer::start().await;
    let client = with_token(&server);
    let directory = test_directory();
    let file = directory.join("private-local-filename.wav");
    std::fs::write(&file, b"file-fixture-content").unwrap();
    for (tool, route) in [
        ("identify_photo", "/api/v1/bird-identification/identify"),
        (
            "identify_audio",
            "/api/v1/bird-identification/audio/identify",
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(header("authorization", "Bearer private-test-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"candidates":[], "calibration_status":"geomodel"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = client.call(tool, json!({"file":file, "latitude":39.9, "longitude":116.4, "captured_at":"2026-10-14"})).await.unwrap();
        assert_eq!(result["calibration_status"], "geomodel");
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"file\""));
        assert!(body.contains("file-fixture-content"));
        assert!(!body.contains("private-local-filename"));
        assert!(!body.contains("private-test-token"));
        assert_eq!(
            request
                .url
                .query_pairs()
                .find(|(k, _)| k == "captured_at")
                .unwrap()
                .1,
            "2026-10-14"
        );
    }
    assert_eq!(
        requests[0]
            .url
            .query_pairs()
            .find(|(k, _)| k == "cloud_calibration")
            .unwrap()
            .1,
        "true"
    );
    assert_eq!(
        requests[1]
            .url
            .query_pairs()
            .find(|(k, _)| k == "window_seconds")
            .unwrap()
            .1,
        "3"
    );
}

#[tokio::test]
async fn invalid_or_oversized_files_send_no_upload() {
    let server = MockServer::start().await;
    let client = with_token(&server);
    let directory = test_directory();
    let empty = directory.join("empty.wav");
    std::fs::write(&empty, b"").unwrap();
    let oversized = directory.join("oversized.wav");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(50 * 1024 * 1024 + 1)
        .unwrap();
    for file in [
        directory.join("missing.wav"),
        directory.clone(),
        empty,
        oversized,
    ] {
        assert!(
            client
                .call("identify_audio", json!({"file":file}))
                .await
                .is_err()
        );
    }
    assert!(
        client
            .call(
                "identify_photo",
                json!({"file":"private-path", "geographic_calibration":"private-weights"})
            )
            .await
            .is_err()
    );
    assert!(
        client
            .call(
                "identify_photo",
                json!({"file":"private-path", "cloud_calibration":false})
            )
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn failed_uploads_are_not_retried_or_redirected() {
    let server = MockServer::start().await;
    let directory = test_directory();
    let file = directory.join("bird.wav");
    std::fs::write(&file, b"audio").unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/bird-identification/audio/identify"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/private-redirect"))
        .expect(1)
        .mount(&server)
        .await;
    let error = with_token(&server)
        .call("identify_audio", json!({"file":file}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("302"));
    assert!(!error.contains("private-test-token"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

struct AdmissionResponder {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    failures: usize,
    status: u16,
    accepted: bool,
    delay: &'static str,
}
impl wiremock::Respond for AdmissionResponder {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call >= self.failures {
            return ResponseTemplate::new(200).set_body_json(json!({"candidates":[]}));
        }
        let response = ResponseTemplate::new(self.status)
            .insert_header("Retry-After", self.delay)
            .set_body_json(json!({"detail":"private-server-message"}));
        if self.accepted {
            response.insert_header("X-Bulbul-Request-Accepted", "false")
        } else {
            response
        }
    }
}

#[tokio::test]
async fn admission_rejection_retries_with_same_file_and_respects_retry_after() {
    let server = MockServer::start().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/api/v1/bird-identification/audio/identify"))
        .respond_with(AdmissionResponder {
            calls: Arc::clone(&calls),
            failures: 1,
            status: 429,
            accepted: true,
            delay: "1",
        })
        .expect(2)
        .mount(&server)
        .await;
    let directory = test_directory();
    let file = directory.join("bird.wav");
    std::fs::write(&file, b"same-file-content").unwrap();
    let started = std::time::Instant::now();
    assert!(
        with_token(&server)
            .call("identify_audio", json!({"file":file, "end_seconds":3}))
            .await
            .is_ok()
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    for request in server.received_requests().await.unwrap() {
        assert!(String::from_utf8_lossy(&request.body).contains("same-file-content"));
        assert_eq!(
            request
                .url
                .query_pairs()
                .find(|(k, _)| k == "end_seconds")
                .unwrap()
                .1,
            "3"
        );
    }
}

#[tokio::test]
async fn admission_retry_stops_after_two_retries() {
    let server = MockServer::start().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/api/v1/bird-identification/identify"))
        .respond_with(AdmissionResponder {
            calls: Arc::clone(&calls),
            failures: usize::MAX,
            status: 429,
            accepted: true,
            delay: "0",
        })
        .expect(3)
        .mount(&server)
        .await;
    let directory = test_directory();
    let file = directory.join("bird.jpg");
    std::fs::write(&file, b"image").unwrap();
    assert!(
        with_token(&server)
            .call("identify_photo", json!({"file":file}))
            .await
            .is_err()
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn uncertain_failures_invalid_delays_and_long_rate_limits_do_not_reupload() {
    for (status, accepted, delay) in [
        (429, false, "0"),
        (503, true, "0"),
        (504, true, "0"),
        (429, true, "bad"),
        (429, true, "300"),
    ] {
        let server = MockServer::start().await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        Mock::given(method("POST"))
            .and(path("/api/v1/bird-identification/audio/identify"))
            .respond_with(AdmissionResponder {
                calls: Arc::clone(&calls),
                failures: usize::MAX,
                status,
                accepted,
                delay,
            })
            .expect(1)
            .mount(&server)
            .await;
        let directory = test_directory();
        let file = directory.join("bird.wav");
        std::fs::write(&file, b"audio").unwrap();
        let error = with_token(&server)
            .call("identify_audio", json!({"file":file}))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-server-message"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
