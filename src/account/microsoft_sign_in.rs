//! Microsoft sign-in: the device-code flow, Xbox Live, XSTS and the Minecraft services, the
//! ownership check and the profile, and renewing a saved sign-in.
use super::*;
use crate::i18n::{tr, trf};

/// The configured Azure app, if any: the run-time variable wins over the build-time one, and an
/// empty value means none.
pub fn client_id() -> Option<String> {
    client_id_from(std::env::var("SUNRISE_MSA_CLIENT_ID").ok())
}

/// [`client_id`] with the run-time variable's value given, so tests don't set it process-wide.
pub(super) fn client_id_from(run_time: Option<String>) -> Option<String> {
    run_time
        .or_else(|| option_env!("SUNRISE_MSA_CLIENT_ID").map(str::to_owned))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
}

/// Where the requests go: the real services, or a mock in tests.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The OAuth 2.0 v2 endpoints for personal accounts (`…/devicecode`, `…/token`).
    pub microsoft: String,
    pub xbox_user: String,
    pub xsts: String,
    /// api.minecraftservices.com.
    pub minecraft: String,
}

impl Endpoints {
    pub fn live() -> Self {
        // Development builds only: `SUNRISE_MSA_MICROSOFT_ENDPOINT` points the device-code and
        // token requests at a local mock, to see and screenshot the sign-in panel.
        let microsoft = std::env::var("SUNRISE_MSA_MICROSOFT_ENDPOINT")
            .ok()
            .filter(|_| cfg!(debug_assertions));
        // The same for the Minecraft services (the profile, its skin and capes), to see and
        // screenshot the Skins tab against a local mock.
        let minecraft = std::env::var("SUNRISE_MINECRAFT_SERVICES_ENDPOINT")
            .ok()
            .filter(|_| cfg!(debug_assertions));
        Self::at(microsoft.as_deref(), minecraft.as_deref())
    }

    /// The live endpoints, taking the development mocks as parameters so the decision does not
    /// depend on the process environment tests share.
    pub(super) fn at(microsoft: Option<&str>, minecraft: Option<&str>) -> Self {
        Self {
            microsoft: microsoft
                .unwrap_or(sunrise_launcher_addresses::MICROSOFT_LOGIN)
                .into(),
            xbox_user: sunrise_launcher_addresses::XBOX_USER_AUTH.into(),
            xsts: sunrise_launcher_addresses::XSTS_AUTH.into(),
            minecraft: minecraft
                .unwrap_or(sunrise_launcher_addresses::MINECRAFT_SERVICES)
                .into(),
        }
    }
}

/// Orders resolved addresses so the two families alternate, the first one's address first (the
/// order RFC 8305 asks a client to try them in). A host whose IPv6 route is dead then costs one
/// short attempt before its IPv4 address is tried, instead of every AAAA record in turn.
pub(super) fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addrs.first().map(SocketAddr::is_ipv6) else {
        return addrs;
    };
    let (mut same, mut other): (Vec<_>, Vec<_>) =
        addrs.into_iter().partition(|a| a.is_ipv6() == first);
    same.reverse();
    other.reverse();
    let mut out = Vec::with_capacity(same.len() + other.len());
    while !same.is_empty() || !other.is_empty() {
        out.extend(same.pop());
        out.extend(other.pop());
    }
    out
}

/// The system's resolver with its answer interleaved (`interleave`).
pub(super) struct Interleaved<R>(pub(super) R);

impl<R: ureq::Resolver> ureq::Resolver for Interleaved<R> {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        self.0.resolve(netloc).map(interleave)
    }
}

struct System;

impl ureq::Resolver for System {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        netloc.to_socket_addrs().map(Iterator::collect)
    }
}

/// Why signing in, or renewing a sign-in, failed. The cases the launcher acts on are their own
/// variants (a refusal only a new sign-in mends marks the account "sign in again"; a saved sign-in
/// that can't be read asks for one); every other failure is its message as the player reads it.
/// Shown, each is the same English text as before: the window translates it by that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInError {
    /// Microsoft refused the sign-in's refresh credential.
    SignInAgain,
    /// The saved sign-in couldn't be read.
    DamagedSignIn,
    /// This build has no Microsoft sign-in.
    NotConfigured,
    DoesNotOwn,
    NoProfile,
    /// The Microsoft account now has another Minecraft profile than the one signed in.
    OtherProfile,
    /// The services couldn't answer just now (no connection, 429, 5xx): not a refusal. Only
    /// this lets Play fall back to a kept token that is still good (#542).
    Unavailable(String),
    Other(String),
}

impl std::fmt::Display for SignInError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SignInAgain => tr(SIGN_IN_AGAIN),
            Self::DamagedSignIn => tr(session::DAMAGED_SIGN_IN),
            Self::NotConfigured => tr(NOT_CONFIGURED),
            Self::DoesNotOwn => tr(DOES_NOT_OWN),
            Self::NoProfile => tr(NO_PROFILE),
            Self::OtherProfile => tr(OTHER_PROFILE),
            Self::Unavailable(message) | Self::Other(message) => message,
        })
    }
}

impl From<String> for SignInError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for SignInError {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

impl From<StoreError> for SignInError {
    fn from(error: StoreError) -> Self {
        Self::Other(error.0)
    }
}

impl From<SignInError> for String {
    fn from(error: SignInError) -> Self {
        error.to_string()
    }
}

/// Keep the short connect timeout and interleaved addresses so a dead IPv6 route
/// falls back to IPv4 before the whole sign-in request times out.
pub(super) fn sign_in_agent(builder: ureq::AgentBuilder) -> ureq::AgentBuilder {
    builder
        .timeout(Duration::from_secs(20))
        .timeout_connect(Duration::from_secs(10))
        .resolver(Interleaved(System))
        .user_agent(crate::proxy::USER_AGENT)
}

pub(super) fn request_to(method: &str, url: &str) -> Result<ureq::Request, String> {
    Ok(crate::proxy::agent("account", url, sign_in_agent)?.request(method, url))
}

/// One request's result: the status and the JSON body (empty when there is none). Transport
/// errors name the service, never the URL's query or the request.
pub(super) fn send(
    request: ureq::Request,
    body: Option<Body>,
    service: &str,
) -> Result<(u16, Value), String> {
    let result = match body {
        None => request.call(),
        Some(Body::Form(fields)) => request.send_form(&fields),
        Some(Body::Json(value)) => request.set("Accept", "application/json").send_json(value),
    };
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(trf(
                "Could not reach {service}. Check your internet connection.",
                &[("service", tr(service))],
            ));
        }
    };
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// [`send`] for the sign-in chain: a request that got no answer (no connection, 429, 5xx) is
/// [`SignInError::Unavailable`], kept apart from the services' refusals.
fn call(
    request: ureq::Request,
    body: Option<Body>,
    service: &str,
) -> Result<(u16, Value), SignInError> {
    let (status, v) = send(request, body, service).map_err(SignInError::Unavailable)?;
    match status {
        429 => Err(SignInError::Unavailable(
            tr("Too many sign-ins in a short time. Wait a minute and try again.").into(),
        )),
        500..=599 => Err(SignInError::Unavailable(trf(
            "{service} isn't answering right now. Try again in a moment.",
            &[("service", tr(service))],
        ))),
        _ => Ok((status, v)),
    }
}

pub(super) enum Body<'a> {
    Form(Vec<(&'a str, &'a str)>),
    Json(Value),
}

// ----- The device-code flow ----------------------------------------------------------------------

/// What the player types at the verification page.
#[derive(Clone, PartialEq)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_at: u64,
    device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    // The device code can be traded for tokens once the player approves: never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish_non_exhaustive()
    }
}

/// The Microsoft tokens; the access token only lasts for the Xbox step.
pub struct MsTokens {
    pub(super) access: String,
    pub(super) refresh: String,
}

impl std::fmt::Debug for MsTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MsTokens(<hidden>)")
    }
}

pub enum Poll {
    Pending,
    /// The server asked to poll less often: wait 5 s more.
    SlowDown,
    Done(MsTokens),
}

pub fn device_code(ep: &Endpoints, client_id: &str) -> Result<DeviceCode, SignInError> {
    let (status, v) = send(
        request_to("POST", &format!("{}/devicecode", ep.microsoft))?,
        Some(Body::Form(vec![("client_id", client_id), ("scope", SCOPE)])),
        "Microsoft",
    )?;
    match (
        status,
        str_field(&v, "user_code"),
        str_field(&v, "device_code"),
    ) {
        (200, Some(user_code), Some(device_code)) => Ok(DeviceCode {
            user_code,
            verification_uri: str_field(&v, "verification_uri")
                .unwrap_or_else(|| sunrise_launcher_addresses::MICROSOFT_LINK.into()),
            interval: v
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .clamp(1, 60),
            // The service's numbers, bounded: Instant + a huge interval panics the sign-in thread.
            expires_at: now().saturating_add(
                v.get("expires_in")
                    .and_then(Value::as_u64)
                    .unwrap_or(900)
                    .min(3600),
            ),
            device_code,
        }),
        _ => Err(oauth_error(&v)),
    }
}

/// The OAuth error codes as messages for the player.
pub(super) fn oauth_error(v: &Value) -> SignInError {
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_declined") => tr("Sign-in was declined in the browser.").into(),
        Some("expired_token") => tr("The sign-in code expired. Start again.").into(),
        Some("bad_verification_code") => {
            tr("The sign-in code was not recognised. Start again.").into()
        }
        Some("invalid_grant") => SignInError::SignInAgain,
        Some("invalid_client" | "unauthorized_client") => {
            tr("Microsoft refused this launcher's app registration (client ID).").into()
        }
        Some(other) => trf("Microsoft sign-in failed ({error}).", &[("error", other)]).into(),
        None => tr("Microsoft sign-in failed.").into(),
    }
}

/// One poll of the token endpoint while the player signs in.
pub fn poll(ep: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Poll, SignInError> {
    let (status, v) = send(
        request_to("POST", &format!("{}/token", ep.microsoft))?,
        Some(Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", &code.device_code),
        ])),
        "Microsoft",
    )?;
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => return Ok(Poll::Pending),
        Some("slow_down") => return Ok(Poll::SlowDown),
        _ => {}
    }
    ms_tokens(status, &v).map(Poll::Done)
}

fn ms_tokens(status: u16, v: &Value) -> Result<MsTokens, SignInError> {
    match (
        status,
        str_field(v, "access_token"),
        str_field(v, "refresh_token"),
    ) {
        (200, Some(access), Some(refresh)) => Ok(MsTokens { access, refresh }),
        _ => Err(oauth_error(v)),
    }
}

/// Wait for the player to finish in the browser, polling as asked, until the code expires or
/// `cancelled` says stop.
pub fn wait_for_sign_in(
    ep: &Endpoints,
    client_id: &str,
    code: &DeviceCode,
    cancelled: &dyn Fn() -> bool,
) -> Result<MsTokens, SignInError> {
    let mut interval = code.interval;
    loop {
        let until = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err(tr("Sign-in cancelled.").into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if now() > code.expires_at {
            return Err(tr("The sign-in code expired. Start again.").into());
        }
        match poll(ep, client_id, code)? {
            Poll::Pending => {}
            Poll::SlowDown => interval = (interval + 5).min(60),
            Poll::Done(tokens) => return Ok(tokens),
        }
    }
}

fn refresh_ms(ep: &Endpoints, client_id: &str, refresh: &str) -> Result<MsTokens, SignInError> {
    let (status, v) = call(
        request_to("POST", &format!("{}/token", ep.microsoft))?,
        Some(Body::Form(vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh),
            ("scope", SCOPE),
        ])),
        "Microsoft",
    )?;
    // A refresh token issued to another app registration (the launcher's client ID changed
    // since the player signed in) is refused as the client's; only a new sign-in mends it.
    match (status, v.get("error").and_then(Value::as_str)) {
        (400..=403, _) | (_, Some("invalid_client" | "unauthorized_client")) => {
            Err(SignInError::SignInAgain)
        }
        _ => ms_tokens(status, &v),
    }
}

// ----- Xbox Live, XSTS, Minecraft ----------------------------------------------------------------

const FAMILY_URL: &str = sunrise_launcher_addresses::MICROSOFT_FAMILY;
const CHILD_ACCOUNT: &str = crate::i18n::tr_key("Microsoft says this is a child account (error {code}). An adult has to add it to a Microsoft family at {url}. If it is an adult account, check its family settings there.");

/// The XSTS `XErr` codes, as the official launcher explains them.
fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => tr("This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one, then try again.").into(),
        2148916235 => tr("Xbox Live is not available in your country or region.").into(),
        2148916236 | 2148916237 => tr("This account needs adult verification on xbox.com before it can play.").into(),
        2148916238 => trf(
            CHILD_ACCOUNT,
            &[("code", &code.to_string()), ("url", FAMILY_URL)],
        ),
        other => trf(
            "Xbox Live refused the sign-in (error {code}).",
            &[("code", &other.to_string())],
        ),
    }
}

/// The Minecraft access token from the Microsoft one, through Xbox Live and XSTS.
struct Minecraft {
    token: String,
    expires_at: u64,
    xuid: Option<String>,
}

fn minecraft_login(
    ep: &Endpoints,
    ms_access: &str,
    progress: &dyn Fn(session::Phase),
) -> Result<Minecraft, SignInError> {
    progress(session::Phase::Xbox);
    let (status, xbl) = call(
        request_to("POST", &ep.xbox_user)?,
        Some(Body::Json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")
        .filter(|_| status == 200)
        .ok_or_else(|| tr("Xbox Live refused the sign-in.").to_string())?;
    let (status, xsts) = call(
        request_to("POST", &ep.xsts)?,
        Some(Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    if let Some(code) = xsts.get("XErr").and_then(Value::as_u64) {
        return Err(xsts_error(code).into());
    }
    let claims = xsts.pointer("/DisplayClaims/xui/0");
    let (Some(xsts_token), Some(uhs)) = (
        str_field(&xsts, "Token").filter(|_| status == 200),
        claims.and_then(|c| str_field(c, "uhs")),
    ) else {
        return Err(tr("Xbox Live refused the sign-in.").into());
    };
    let xuid = claims.and_then(|c| str_field(c, "xid"));
    progress(session::Phase::Minecraft);
    let (token, expires_in) = minecraft_token(ep, &format!("XBL3.0 x={uhs};{xsts_token}"))?;
    Ok(Minecraft {
        token,
        expires_at: now().saturating_add(expires_in.min(7 * 86_400)),
        xuid,
    })
}

/// The Minecraft access token and its lifetime from the XSTS one (`XBL3.0 x=<uhs>;<token>`).
/// `launcher/login` first: a new account's token from `login_with_xbox` names no profile, so
/// joining an online-mode server answers "Profile not found" (found and diagnosed by
/// martipops). `login_with_xbox` stays the fallback, so a sign-in that worked before still does
/// when the other refuses.
fn minecraft_token(ep: &Endpoints, xtoken: &str) -> Result<(String, u64), SignInError> {
    let lifetime = |v: &Value| v.get("expires_in").and_then(Value::as_u64).unwrap_or(86400);
    let launcher = send(
        request_to("POST", &format!("{}/launcher/login", ep.minecraft))?,
        Some(Body::Json(
            json!({ "xtoken": xtoken, "platform": "PC_LAUNCHER" }),
        )),
        "the Minecraft services",
    );
    if let Ok((200, v)) = &launcher {
        if let Some(token) = str_field(v, "access_token") {
            return Ok((token, lifetime(v)));
        }
    }
    let (status, mc) = call(
        request_to(
            "POST",
            &format!("{}/authentication/login_with_xbox", ep.minecraft),
        )?,
        Some(Body::Json(json!({ "identityToken": xtoken }))),
        "the Minecraft services",
    )?;
    match (status, str_field(&mc, "access_token")) {
        (200, Some(token)) => Ok((token, lifetime(&mc))),
        // New Azure apps are refused until Mojang approves them (Minecraft API access).
        (403, _) => Err(tr(
            "The Minecraft services refused this launcher: its app is not approved by Mojang yet.",
        )
        .into()),
        (429, _) => {
            Err(tr("Too many sign-ins in a short time. Wait a minute and try again.").into())
        }
        _ => Err(tr("The Minecraft services refused the sign-in.").into()),
    }
}

pub(super) fn bearer(request: ureq::Request, token: &str) -> ureq::Request {
    request.set("Authorization", &format!("Bearer {token}"))
}

/// Whether the account owns Minecraft: Java Edition (Game Pass included).
fn owns_game(ep: &Endpoints, token: &str) -> Result<bool, SignInError> {
    let (status, v) = call(
        bearer(
            request_to("GET", &format!("{}/entitlements/mcstore", ep.minecraft))?,
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if matches!(status, 401 | 403) {
        return Err(SignInError::SignInAgain);
    }
    if status != 200 {
        return Err(tr("Could not check which games this account owns.").into());
    }
    Ok(v.get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("name").and_then(Value::as_str),
                    Some("game_minecraft" | "product_minecraft")
                )
            })
        }))
}

/// A Minecraft profile, as the launcher shows and remembers it (no secrets).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Account {
    /// The profile UUID without dashes.
    pub uuid: String,
    pub name: String,
    /// The active skin's URL (textures.minecraft.net) and model ("classic" or "slim").
    #[serde(default)]
    pub skin: Option<(String, String)>,
    #[serde(default)]
    pub cape: Option<String>,
}

/// The profile; none when the account has none (it doesn't own the game, or never made one).
fn profile(ep: &Endpoints, token: &str) -> Result<Option<Account>, SignInError> {
    let (status, v) = call(
        bearer(
            request_to("GET", &format!("{}/minecraft/profile", ep.minecraft))?,
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if matches!(status, 401 | 403) {
        return Err(SignInError::SignInAgain);
    }
    if status == 404 {
        return Ok(None);
    }
    let (200, Some(uuid), Some(name)) = (status, str_field(&v, "id"), str_field(&v, "name")) else {
        return Err(tr("Could not read the Minecraft profile.").into());
    };
    let active = |key: &str| {
        v.get(key)
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|s| s.get("state").and_then(Value::as_str) == Some("ACTIVE"))
            })
            .cloned()
    };
    Ok(Some(Account {
        uuid: uuid.replace('-', "").to_lowercase(),
        name,
        skin: active("skins").and_then(|s| {
            Some((
                str_field(&s, "url")?,
                str_field(&s, "variant")
                    .unwrap_or_else(|| "CLASSIC".into())
                    .to_lowercase(),
            ))
        }),
        cape: active("capes").and_then(|c| str_field(&c, "url")),
    }))
}

/// What the keychain keeps for one account.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secrets {
    pub(super) refresh_token: String,
    #[serde(default)]
    pub(super) sign_in_required: bool,
    pub(super) minecraft_token: String,
    pub(super) minecraft_expires_at: u64,
    #[serde(default)]
    pub(super) xuid: Option<String>,
    /// The last answer the Minecraft services gave: whether the account owns the game. It is
    /// kept here, beside the tokens, so that no plain file can say yes. Accounts saved before it
    /// existed passed the same check at sign-in, which refuses everyone else.
    #[serde(default = "owned_at_sign_in")]
    pub(super) owns: bool,
}
fn owned_at_sign_in() -> bool {
    true
}
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets(<hidden>)")
    }
}

/// From Microsoft tokens to a ready account: Xbox, XSTS, Minecraft, ownership, profile.
pub fn finish_sign_in(ep: &Endpoints, ms: MsTokens) -> Result<(Account, Secrets), SignInError> {
    finish_sign_in_with_progress(ep, ms, &|_| {})
}

pub fn finish_sign_in_with_progress(
    ep: &Endpoints,
    ms: MsTokens,
    progress: &dyn Fn(session::Phase),
) -> Result<(Account, Secrets), SignInError> {
    let mc = minecraft_login(ep, &ms.access, progress)?;
    progress(session::Phase::Ownership);
    if !owns_game(ep, &mc.token)? {
        return Err(SignInError::DoesNotOwn);
    }
    progress(session::Phase::Profile);
    let account = profile(ep, &mc.token)?.ok_or(SignInError::NoProfile)?;
    Ok((account, Secrets::new(ms.refresh, mc)))
}

impl Secrets {
    fn new(refresh_token: String, mc: Minecraft) -> Self {
        Self {
            refresh_token,
            sign_in_required: false,
            minecraft_token: mc.token,
            minecraft_expires_at: mc.expires_at,
            xuid: mc.xuid,
            owns: true,
        }
    }
}

/// What asking the Minecraft services again found for one account.
pub(super) enum Check {
    /// It owns the game: its profile as it is now (the name may have changed) and its tokens.
    Owns(Account, Secrets),
    /// The services said it doesn't own the game (or has no profile any more).
    DoesNot(Secrets),
    /// No answer (no network, a sign-in that can't be renewed): the last answer stands.
    NoAnswer(SignInError, Secrets),
}

/// Asks whether `account` owns the game, with a Minecraft token renewed through the refresh token
/// first when it has less than an hour left. The secrets come back renewed whatever the answer.
pub(super) fn check(
    ep: &Endpoints,
    client_id: Option<&str>,
    account: &Account,
    secrets: Secrets,
) -> Check {
    check_with(ep, client_id, account, secrets, false, &|_| {})
}

/// Whether the Minecraft token is due for renewal: the earlier of the kept expiry and the
/// token's own `exp` (#618) is within [`RENEW_BEFORE`], so a token whose claim has passed is
/// renewed while the services answer rather than sent and refused.
fn renew_due(secrets: &Secrets) -> bool {
    let expires_at = super::game_launch::token_exp(&secrets.minecraft_token)
        .map_or(secrets.minecraft_expires_at, |exp| {
            exp.min(secrets.minecraft_expires_at)
        });
    expires_at <= now() + RENEW_BEFORE
}

/// A new Minecraft token through the refresh token, which Microsoft may rotate in `secrets`
/// even when a later step fails. The token hasn't passed the ownership and profile checks yet.
fn renew(
    ep: &Endpoints,
    client_id: Option<&str>,
    secrets: &mut Secrets,
    progress: &dyn Fn(session::Phase),
) -> Result<Minecraft, SignInError> {
    progress(session::Phase::Microsoft);
    let ms = client_id
        .ok_or(SignInError::NotConfigured)
        .and_then(|id| refresh_ms(ep, id, &secrets.refresh_token))
        .inspect_err(|e| secrets.sign_in_required = matches!(e, SignInError::SignInAgain))?;
    // Microsoft can rotate its refresh credential even if a later service fails.
    secrets.refresh_token = ms.refresh;
    minecraft_login(ep, &ms.access, progress)
}

pub(super) fn check_with(
    ep: &Endpoints,
    client_id: Option<&str>,
    account: &Account,
    mut secrets: Secrets,
    force: bool,
    progress: &dyn Fn(session::Phase),
) -> Check {
    if secrets.sign_in_required {
        return Check::NoAnswer(SignInError::SignInAgain, secrets);
    }
    // The credentials kept before this check. Any answer short of "owns it, same profile" goes
    // back to them (with Microsoft's rotated refresh token and the refusal), so a renewed
    // Minecraft token is kept and played with only once it has passed both checks.
    let mut kept = secrets.clone();
    if force || renew_due(&secrets) {
        let renewed = renew(ep, client_id, &mut secrets, progress);
        kept.refresh_token.clone_from(&secrets.refresh_token);
        match renewed {
            Ok(mc) => {
                secrets = Secrets {
                    owns: secrets.owns,
                    ..Secrets::new(secrets.refresh_token, mc)
                }
            }
            Err(e) => return Check::NoAnswer(e, secrets),
        }
    }
    let token = &secrets.minecraft_token;
    progress(session::Phase::Ownership);
    match owns_game(ep, token).and_then(|owns| {
        if !owns {
            return Ok(None);
        }
        progress(session::Phase::Profile);
        profile(ep, token)
    }) {
        Err(e) => {
            kept.sign_in_required = matches!(e, SignInError::SignInAgain);
            Check::NoAnswer(e, kept)
        }
        Ok(None) => Check::DoesNot(Secrets {
            owns: false,
            ..kept
        }),
        Ok(Some(p)) if p.uuid != account.uuid => Check::NoAnswer(
            SignInError::OtherProfile,
            Secrets {
                sign_in_required: true,
                ..kept
            },
        ),
        Ok(Some(p)) => Check::Owns(
            p,
            Secrets {
                owns: true,
                ..secrets
            },
        ),
    }
}
