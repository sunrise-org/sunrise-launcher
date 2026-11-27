//! The sign-in chain against a local mock of the Microsoft, Xbox Live and Minecraft endpoints
//! (recorded response shapes, mock tokens only), and the keychain abstraction in memory.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Instant;

pub(crate) const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

/// What the mock answers and what it was asked.
#[derive(Default)]
struct Mock {
    /// Polls left before the token endpoint stops saying authorization_pending.
    pending: usize,
    refresh_refused: Option<u16>,
    entitlements_refused: Option<u16>,
    xerr: Option<u64>,
    owns_game: bool,
    no_profile: bool,
    /// What `launcher/login` answers instead of a token (an older service, an outage).
    launcher_login_refused: Option<u16>,
    /// The last skin sent asked for slim arms.
    slim: bool,
    /// The cape a PUT or DELETE on `capes/active` left worn; unset: the profile's own (c2).
    worn: Option<Option<String>>,
    /// What a skin upload is answered with instead of the profile: a status and an empty body
    /// (an acknowledgement with nothing usable in it).
    skin_answer_empty: Option<u16>,
    /// The same for a cape change.
    cape_answer_empty: Option<u16>,
    /// A skin upload is acknowledged but changes nothing (the profile keeps the skin before).
    upload_ignored: bool,
    /// A cape change is acknowledged but changes nothing.
    cape_ignored: bool,
    /// The account wears a default skin (no skin on its profile) until a skin is uploaded.
    no_skin: bool,
    /// What the next profile readings answer, in turn, before they answer with the profile: 0
    /// the profile, 200 a body with a malformed `capes` field, any other status an error page.
    profile_answers: std::collections::VecDeque<u16>,
    /// The worn skin's picture; unset: `texture/abc`. Each upload gets a new one.
    skin_url: Option<String>,
    /// Every skin upload, as the services would parse it: its content type's boundary and its
    /// form parts, by name.
    uploads: Vec<Upload>,
    /// Every request: method, path and body.
    log: Vec<(String, String, String)>,
}

/// One skin upload's multipart form, parsed from the bytes sent.
#[derive(Debug, Default)]
struct Upload {
    variant: String,
    file_name: String,
    file_type: String,
    file: Vec<u8>,
    /// The body closed with `--boundary--` and nothing after it but the line end.
    closed: bool,
}

/// Parses a multipart/form-data body with `boundary`, as a server does: the parts between
/// `--boundary` lines, each with its headers, a blank line and its bytes.
fn parse_upload(boundary: &str, body: &[u8]) -> Upload {
    let delimiter = format!("--{boundary}").into_bytes();
    let find = |from: usize, needle: &[u8]| {
        body[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|i| i + from)
    };
    let mut upload = Upload::default();
    let mut at = find(0, &delimiter).expect("a first boundary");
    loop {
        at += delimiter.len();
        if body[at..].starts_with(b"--") {
            upload.closed = &body[at + 2..] == b"\r\n";
            return upload;
        }
        assert!(
            body[at..].starts_with(b"\r\n"),
            "a line end after the boundary"
        );
        let start = at + 2;
        let head_end = find(start, b"\r\n\r\n").expect("the part's headers");
        let head = String::from_utf8(body[start..head_end].to_vec()).unwrap();
        let next =
            find(head_end, &[b"\r\n".as_slice(), &delimiter].concat()).expect("the next boundary");
        let content = body[head_end + 4..next].to_vec();
        let name = head
            .split("name=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or_default();
        match name {
            "variant" => upload.variant = String::from_utf8(content).unwrap(),
            "file" => {
                upload.file_name = head
                    .split("filename=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap_or_default()
                    .to_owned();
                upload.file_type = head
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Type: "))
                    .unwrap_or_default()
                    .to_owned();
                upload.file = content;
            }
            other => panic!("an unexpected part {other}"),
        }
        at = next + 2;
    }
}

/// The mock's profile, its capes worn as `mock.worn` says.
fn mock_profile(mock: &Mock) -> Value {
    let worn = mock.worn.clone().unwrap_or(Some("c2".into()));
    let cape = |id: &str, alias: &str| {
        let state = if worn.as_deref() == Some(id) {
            "ACTIVE"
        } else {
            "INACTIVE"
        };
        json!({"id": id, "state": state, "url": format!("http://textures.minecraft.net/texture/{id}"), "alias": alias})
    };
    let skins = if mock.no_skin && mock.skin_url.is_none() {
        json!([])
    } else {
        json!([{"id": "s1", "state": "ACTIVE",
                "url": mock.skin_url.as_deref().unwrap_or("http://textures.minecraft.net/texture/abc"),
                "variant": if mock.slim || mock.worn.is_none() { "SLIM" } else { "CLASSIC" }}])
    };
    json!({
        "id": UUID, "name": "Notch",
        "skins": skins,
        "capes": [cape("c1", "Migrator"), cape("c2", "Vanilla")],
    })
}

fn reply(status: u16, body: Value) -> (u16, String) {
    (status, body.to_string())
}

fn route(mock: &mut Mock, method: &str, path: &str, auth: &str, body: &str) -> (u16, String) {
    let form = |key: &str| {
        body.split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.replace('+', " ").replace("%3A", ":").replace("%20", " "))
    };
    match (method, path) {
        ("POST", "/ms/devicecode") => {
            assert_eq!(form("client_id").as_deref(), Some("test-client"));
            assert_eq!(form("scope").as_deref(), Some(SCOPE));
            reply(
                200,
                json!({"user_code": "ABCD-EFGH", "device_code": "dev-1", "verification_uri": "https://www.microsoft.com/link", "expires_in": 900, "interval": 1}),
            )
        }
        ("POST", "/ms/token") => match form("grant_type").as_deref() {
            Some("urn:ietf:params:oauth:grant-type:device_code") if mock.pending > 0 => {
                mock.pending -= 1;
                reply(400, json!({"error": "authorization_pending"}))
            }
            Some("urn:ietf:params:oauth:grant-type:device_code") => {
                assert_eq!(form("device_code").as_deref(), Some("dev-1"));
                reply(
                    200,
                    json!({"access_token": "ms-access-1", "refresh_token": "ms-refresh-1", "expires_in": 3600}),
                )
            }
            Some("refresh_token") if mock.refresh_refused.is_some() => {
                reply(mock.refresh_refused.unwrap(), json!({}))
            }
            Some("refresh_token") if form("refresh_token").as_deref() == Some("ms-refresh-1") => {
                reply(
                    200,
                    json!({"access_token": "ms-access-2", "refresh_token": "ms-refresh-2"}),
                )
            }
            Some("refresh_token") if form("refresh_token").as_deref() == Some("other-app") => {
                reply(400, json!({"error": "unauthorized_client"}))
            }
            _ => reply(400, json!({"error": "invalid_grant"})),
        },
        ("POST", "/xbl") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert!(v["Properties"]["RpsTicket"]
                .as_str()
                .unwrap()
                .starts_with("d=ms-access-"));
            assert_eq!(v["RelyingParty"], "http://auth.xboxlive.com");
            reply(
                200,
                json!({"Token": "xbl-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1"}]}}),
            )
        }
        ("POST", "/xsts") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["Properties"]["UserTokens"][0], "xbl-token");
            assert_eq!(v["RelyingParty"], "rp://api.minecraftservices.com/");
            match mock.xerr {
                Some(code) => reply(401, json!({"Identity": "0", "XErr": code, "Message": ""})),
                None => reply(
                    200,
                    json!({"Token": "xsts-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1", "xid": "2535400000000000"}]}}),
                ),
            }
        }
        ("POST", "/mc/launcher/login") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["xtoken"], "XBL3.0 x=uhs-1;xsts-token");
            assert_eq!(v["platform"], "PC_LAUNCHER");
            match mock.launcher_login_refused {
                Some(status) => reply(
                    status,
                    json!({"path": "/launcher/login", "error": "UNAUTHORIZED"}),
                ),
                None => reply(
                    200,
                    json!({"access_token": "mc-token", "token_type": "Bearer", "expires_in": 86400}),
                ),
            }
        }
        ("POST", "/mc/authentication/login_with_xbox") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["identityToken"], "XBL3.0 x=uhs-1;xsts-token");
            reply(
                200,
                json!({"username": "mock", "access_token": "mc-token", "token_type": "Bearer", "expires_in": 86400}),
            )
        }
        ("GET", "/mc/entitlements/mcstore") if mock.entitlements_refused.is_some() => {
            reply(mock.entitlements_refused.unwrap(), json!({}))
        }
        ("GET", "/mc/entitlements/mcstore") => {
            assert_eq!(auth, "Bearer mc-token");
            let items = if mock.owns_game {
                json!([{"name": "product_minecraft", "signature": "x"}, {"name": "game_minecraft", "signature": "x"}])
            } else {
                json!([])
            };
            reply(200, json!({"items": items, "signature": "x", "keyId": "1"}))
        }
        ("GET", "/mc/minecraft/profile") if mock.no_profile => reply(
            404,
            json!({"path": "/minecraft/profile", "errorType": "NOT_FOUND", "error": "NOT_FOUND"}),
        ),
        ("GET", "/mc/minecraft/profile") => match mock.profile_answers.pop_front() {
            Some(200) => reply(200, json!({"id": UUID, "skins": [], "capes": "none"})),
            Some(0) | None => reply(200, mock_profile(mock)),
            Some(status) => reply(status, json!({"error": "TEMPORARILY_UNAVAILABLE"})),
        },
        ("PUT", "/mc/minecraft/profile/capes/active") => {
            assert_eq!(auth, "Bearer mc-token");
            let v: Value = serde_json::from_str(body).unwrap();
            match v["capeId"].as_str() {
                Some(id @ ("c1" | "c2")) => {
                    if !mock.cape_ignored {
                        mock.worn = Some(Some(id.into()));
                    }
                    match mock.cape_answer_empty {
                        Some(status) => (status, String::new()),
                        None => reply(200, mock_profile(mock)),
                    }
                }
                _ => reply(400, json!({"errorType": "BAD_REQUEST"})),
            }
        }
        ("DELETE", "/mc/minecraft/profile/capes/active") => {
            assert_eq!(auth, "Bearer mc-token");
            if !mock.cape_ignored {
                mock.worn = Some(None);
            }
            match mock.cape_answer_empty {
                Some(status) => (status, String::new()),
                None => reply(200, mock_profile(mock)),
            }
        }
        ("POST", "/mc/minecraft/profile/skins") => {
            assert_eq!(auth, "Bearer mc-token");
            let upload = mock.uploads.last().expect("the upload's form");
            let slim = upload.variant == "slim";
            let skin = image::load_from_memory_with_format(&upload.file, image::ImageFormat::Png);
            if !upload.closed || !matches!(skin, Ok(ref s) if s.width() == 64) {
                return reply(400, json!({"errorType": "BAD_REQUEST"}));
            }
            if !mock.upload_ignored {
                mock.slim = slim;
                mock.skin_url = Some(format!(
                    "http://textures.minecraft.net/texture/up{}",
                    mock.uploads.len()
                ));
            }
            match mock.skin_answer_empty {
                Some(status) => (status, String::new()),
                None => reply(200, mock_profile(mock)),
            }
        }
        _ => reply(404, json!({"error": "NotFound"})),
    }
}

/// A one-thread HTTP/1.1 server: one request per connection.
fn serve(mock: Arc<Mutex<Mock>>) -> Endpoints {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_owned(),
                parts.next().unwrap_or("").to_owned(),
            );
            let (mut length, mut auth, mut kind) = (0, String::new(), String::new());
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (k, v) = header.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "authorization" => auth = v.trim().to_owned(),
                    "content-type" => kind = v.trim().to_owned(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let raw = body;
            let body = String::from_utf8_lossy(&raw).into_owned();
            let (status, text) = {
                let mut m = mock.lock().unwrap();
                if let Some(boundary) = kind.strip_prefix("multipart/form-data; boundary=") {
                    m.uploads.push(parse_upload(boundary, &raw));
                }
                // A bare `GET /` is a local tool probing every new port (T3 Code does, on the Mac
                // mini), not the launcher: no endpoint here is the root.
                if (method.as_str(), path.as_str()) != ("GET", "/") {
                    m.log.push((method.clone(), path.clone(), body.clone()));
                }
                route(&mut m, &method, &path, &auth, &body)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    Endpoints {
        microsoft: format!("{base}/ms"),
        xbox_user: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        minecraft: format!("{base}/mc"),
    }
}

fn mock(f: impl FnOnce(&mut Mock)) -> (Arc<Mutex<Mock>>, Endpoints) {
    let mut m = Mock {
        owns_game: true,
        ..Default::default()
    };
    f(&mut m);
    let m = Arc::new(Mutex::new(m));
    let ep = serve(m.clone());
    (m, ep)
}

#[cfg(unix)]
pub(crate) fn minecraft_services_mock() -> String {
    mock(|_| {}).1.minecraft
}

fn notch() -> Account {
    Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    }
}

/// A signed-in account's secrets: a token good for two hours, and the last ownership answer.
fn secrets(token: &str, owns: bool) -> Secrets {
    Secrets {
        refresh_token: "ms-refresh-1".into(),
        sign_in_required: false,
        minecraft_token: token.into(),
        minecraft_expires_at: now() + 7200,
        xuid: None,
        owns,
    }
}

/// Nothing listens there: every request fails as with no network.
fn offline() -> Endpoints {
    Endpoints {
        microsoft: "http://127.0.0.1:9/ms".into(),
        xbox_user: "http://127.0.0.1:9/xbl".into(),
        xsts: "http://127.0.0.1:9/xsts".into(),
        minecraft: "http://127.0.0.1:9/mc".into(),
    }
}

/// An accounts.json with Notch signed in and playing, his secrets in `store`.
fn signed_in(store: &MemoryStore, secrets: &Secrets) -> Accounts {
    let mut accounts = Accounts::default();
    accounts.add(store, notch(), secrets).unwrap();
    accounts
}

/// An accounts.json that is never there: a refresh in these tests saves no profile, and never
/// touches the developer's own launcher data.
fn nowhere() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "sunrise-launcher-no-accounts-{}",
            std::process::id()
        ))
        .join("accounts.json")
}

fn kept(store: &MemoryStore) -> Secrets {
    secrets_in(store, UUID).unwrap()
}

/// A keychain that reads but refuses to keep anything new (#3249).
struct Refusing(MemoryStore);
impl SecretStore for Refusing {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.0.get(key)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
        Err("the keyring is locked".into())
    }
    fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.0.delete(key)
    }
    fn persistent(&self) -> bool {
        true
    }
    fn operations(&self) -> &Mutex<()> {
        self.0.operations()
    }
}

/// Keychain items in memory; writes fail once `room` items are kept, as a full Credential Manager
/// refuses new items while it still overwrites the ones it has.
struct FullItems {
    items: Mutex<HashMap<String, String>>,
    room: usize,
}
impl Items for FullItems {
    fn read(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self.items.lock().unwrap().get(key).cloned())
    }
    fn write(&self, key: &str, piece: &str) -> Result<(), StoreError> {
        let mut items = self.items.lock().unwrap();
        if !items.contains_key(key) && items.len() >= self.room {
            return Err(credential_manager_says(8).unwrap().into());
        }
        items.insert(key.into(), piece.into());
        Ok(())
    }
    fn remove(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self.items.lock().unwrap().remove(key).is_some())
    }
}

/// Items that answer `left` removals, then refuse them, as a keychain denying a delete does.
struct RefusedDeletes {
    items: Mutex<HashMap<String, String>>,
    left: Mutex<usize>,
}
impl Items for RefusedDeletes {
    fn read(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self.items.lock().unwrap().get(key).cloned())
    }
    fn write(&self, key: &str, piece: &str) -> Result<(), StoreError> {
        self.items.lock().unwrap().insert(key.into(), piece.into());
        Ok(())
    }
    fn remove(&self, key: &str) -> Result<bool, StoreError> {
        let mut left = self.left.lock().unwrap();
        if *left == 0 {
            return Err("the keychain refused the delete".into());
        }
        *left -= 1;
        Ok(self.items.lock().unwrap().remove(key).is_some())
    }
}

/// The keys `has_owner` asked for, in order (each read can ask for the keychain on macOS).
struct Reading<'a>(&'a MemoryStore, Mutex<Vec<String>>);
impl SecretStore for Reading<'_> {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.1.lock().unwrap().push(key.into());
        self.0.get(key)
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), StoreError> {
        self.0.set(key, secret)
    }
    fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.0.delete(key)
    }
    fn persistent(&self) -> bool {
        false
    }
    fn operations(&self) -> &Mutex<()> {
        self.0.operations()
    }
}

/// A secrets file of the test's own, gone first.
fn secrets_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-secrets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("launcher").join(SECRETS_FILE)
}

fn remove_secrets_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

/// The files beside the secrets file, by name.
fn files_beside(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const WRITER: &str = "SUNRISE_TEST_SECRETS_WRITER";

/// One of the processes [`processes_sharing_the_secrets_file_lose_nothing`] starts: does nothing
/// unless it gives the file and the writer's number.
#[test]
#[ignore = "a child process of processes_sharing_the_secrets_file_lose_nothing"]
fn secrets_file_writer() {
    let Some(spec) = std::env::var_os(WRITER) else {
        return;
    };
    let spec = spec.into_string().unwrap();
    let (w, path) = spec.split_once(':').unwrap();
    let store = FileStore::new(PathBuf::from(path));
    for i in 0..25 {
        store
            .set(&format!("{w}-{i}"), &format!("secret-{w}-{i}"))
            .unwrap();
    }
    if w == "0" {
        store.delete("signed-out").unwrap();
    }
}

/// A keychain that can be away (a Secret Service that didn't start, a dismissed unlock prompt),
/// refuse some keys, or hold every call while `prompt` is held (an unlock prompt left open).
#[derive(Default)]
struct Keychain {
    kept: MemoryStore,
    away: std::sync::atomic::AtomicBool,
    refuses: Mutex<Vec<String>>,
    prompt: Mutex<()>,
    /// Calls that reached it: each one could have been a prompt.
    asked: std::sync::atomic::AtomicUsize,
    /// Run once, right after the next call it didn't answer.
    after_away: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Run once, while it takes the next secret.
    while_taking: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Keychain {
    fn answer(&self) -> Result<(), String> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        drop(self.prompt.lock().unwrap());
        if self.away.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(then) = self.after_away.lock().unwrap().take() {
                then();
            }
            return Err("The name org.freedesktop.secrets was not provided".into());
        }
        Ok(())
    }
    fn set_away(&self, away: bool) {
        self.away.store(away, std::sync::atomic::Ordering::Relaxed);
    }
    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// One keychain seen by several launchers.
struct Shared(Arc<Keychain>);

impl SecretStore for Shared {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.0.answer()?;
        self.0.kept.get(key)
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), StoreError> {
        self.0.answer()?;
        if self.0.refuses.lock().unwrap().iter().any(|k| k == key) {
            return Err("the keychain is full".into());
        }
        if let Some(meanwhile) = self.0.while_taking.lock().unwrap().take() {
            meanwhile();
        }
        self.0.kept.set(key, secret)
    }
    fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.0.answer()?;
        self.0.kept.delete(key)
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// A launcher starting with this keychain and secrets file.
fn launcher_with(keychain: &Arc<Keychain>, path: &Path) -> Layered {
    Layered::new(
        Box::new(Shared(keychain.clone())),
        FileStore::new(path.to_path_buf()),
    )
}

/// A start handing the file's A to the keychain, held up mid-way by an unlock prompt, while a
/// launcher already running does `meanwhile` to A. The running one waits for the hand-over instead
/// of racing it, and goes on once the prompt is answered.
fn hand_over_racing(
    name: &str,
    meanwhile: impl FnOnce(&Layered) + Send + 'static,
) -> (Arc<Keychain>, PathBuf) {
    let path = secrets_file(name);
    let keychain = Arc::new(Keychain::default());
    // Started before the file had anything: it asked the keychain nothing.
    let running = launcher_with(&keychain, &path);
    // A run the keychain missed kept A in the file.
    FileStore::new(path.clone()).set("A", "old-a").unwrap();
    let prompt = keychain.prompt.lock().unwrap();
    let asked = keychain.asked();
    let starting = {
        let (keychain, path) = (keychain.clone(), path.clone());
        std::thread::spawn(move || drop(launcher_with(&keychain, &path)))
    };
    while keychain.asked() == asked {
        std::thread::sleep(Duration::from_millis(1));
    }
    let racing = std::thread::spawn(move || meanwhile(&running));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        keychain.asked(),
        asked + 1,
        "the running launcher reached the keychain during the hand-over"
    );
    drop(prompt);
    starting.join().unwrap();
    racing.join().unwrap();
    (keychain, path)
}

/// A keyring that never answers until the test lets it go: a stuck Secret Service daemon
/// (#12179).
struct Silent {
    hung: Arc<Mutex<()>>,
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

impl Silent {
    fn answer(&self) {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        drop(self.hung.lock().unwrap());
    }
}

impl SecretStore for Silent {
    fn get(&self, _: &str) -> Result<Option<String>, StoreError> {
        self.answer();
        Ok(None)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
        self.answer();
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        self.answer();
        Ok(())
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// How long the tests' keyrings get: short, the behaviour is the same as with [`KEYCHAIN_WAIT`].
const SILENT_WAIT: Duration = Duration::from_millis(300);

struct Fixed(Vec<SocketAddr>);

impl ureq::Resolver for Fixed {
    fn resolve(&self, _: &str) -> std::io::Result<Vec<SocketAddr>> {
        Ok(self.0.clone())
    }
}

/// A `w` by `h` PNG, each texel coloured by its place.
fn png(w: u32, h: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 7, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A Save's tools against the mock: `pictures` are what Mojang's texture server has, besides
/// each skin uploaded to the mock (at `texture/upN`, the picture sent).
#[derive(Default)]
struct LookBench {
    library: Option<tempfile::TempDir>,
    pictures: HashMap<String, Vec<u8>>,
    fetched: std::cell::RefCell<Vec<String>>,
    records: std::cell::RefCell<Vec<look::Record>>,
}

impl LookBench {
    fn new() -> LookBench {
        LookBench {
            library: Some(tempfile::tempdir().unwrap()),
            ..Default::default()
        }
    }

    fn dir(&self) -> &Path {
        self.library.as_ref().unwrap().path()
    }

    fn save(&self, m: &Arc<Mutex<Mock>>, ep: &Endpoints, request: &look::Request) -> look::Outcome {
        let fetch = |url: &str| {
            self.fetched.borrow_mut().push(url.to_owned());
            self.pictures.get(url).cloned().or_else(|| {
                let n: usize = url
                    .strip_prefix("http://textures.minecraft.net/texture/up")?
                    .parse()
                    .ok()?;
                let sent = m
                    .lock()
                    .unwrap()
                    .uploads
                    .get(n.checked_sub(1)?)?
                    .file
                    .clone();
                Some(served_copy(&sent))
            })
        };
        let record = |record: &look::Record| {
            self.records.borrow_mut().push(record.clone());
            Ok(())
        };
        look::save(
            request,
            &look::Tools {
                ep,
                token: Ok("mc-token".into()),
                library: Ok(self.dir().to_owned()),
                fetch: &fetch,
                record: &record,
            },
        )
    }
}

/// The texture server's copy of a skin uploaded: the same texels in another PNG, as Mojang
/// re-encodes what it is sent (another compression and filter, and nothing kept under the
/// texels left clear).
fn served_copy(sent: &[u8]) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut image = image::load_from_memory_with_format(sent, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8();
    for texel in image.pixels_mut() {
        if texel.0[3] == 0 {
            texel.0 = [0; 4];
        }
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Paeth,
    )
    .write_image(&image, 64, image.height(), image::ExtendedColorType::Rgba8)
    .unwrap();
    out
}

/// A 64 by 64 skin, one texel of it `n`.
fn skin_png(n: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba(if (x, y) == (0, 0) {
            [n, 1, 1, 255]
        } else {
            [x as u8, y as u8, 7, 255]
        })
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn skin_request(png: Vec<u8>, slim: bool, cape: Option<Option<&str>>) -> look::Request {
    look::Request {
        uuid: UUID.into(),
        skin: Some(look::SkinSent {
            picture: look::Picture::New { png, saved: None },
            slim,
        }),
        cape: cape.map(|c| c.map(str::to_owned)),
    }
}

/// Only the arms of the skin the page shows as worn.
fn arms_request(worn: look::WornPicture, slim: bool) -> look::Request {
    look::Request {
        uuid: UUID.into(),
        skin: Some(look::SkinSent {
            picture: look::Picture::Worn(worn),
            slim,
        }),
        cape: None,
    }
}

fn record(skin: Option<Option<(&str, &str)>>, cape: Option<Option<&str>>) -> look::Record {
    look::Record {
        skin: skin.map(|s| s.map(|(url, model)| (url.into(), model.into()))),
        cape: cape.map(|c| c.map(str::to_owned)),
    }
}

const ABC: &str = "http://textures.minecraft.net/texture/abc";
const UP1: &str = "http://textures.minecraft.net/texture/up1";
const C1: &str = "http://textures.minecraft.net/texture/c1";

fn methods(m: &Arc<Mutex<Mock>>) -> Vec<String> {
    m.lock()
        .unwrap()
        .log
        .iter()
        .map(|(method, _, _)| method.clone())
        .collect()
}

mod cape_choice;
mod file_secrets;
mod hand_over;
mod keychain;
mod playing;
mod saved_list;
mod saving_looks;
mod signing_in;
mod store_layers;
