//! Capes: the ones a Minecraft account owns and which of them it wears (the Minecraft services'
//! profile), and the player's OptiFine cape.
//!
//! A Minecraft cape is given by Mojang and listed on the profile, `ACTIVE` or `INACTIVE`; the
//! account wears one (`PUT /minecraft/profile/capes/active` with its id) or none (`DELETE` on the
//! same path), and every client and server shows that one, since it travels in the signed profile.
//!
//! An OptiFine cape is a picture OptiFine's cape server keeps under the player's name
//! (`http://s.optifine.net/capes/<name>.png`, plain HTTP: the server has no HTTPS). Only clients
//! that ask that server show it.
use super::{bearer, json, request_to, send, str_field, Body, Endpoints, Value};
use std::io::Read;
use std::time::Duration;

/// OptiFine's cape server.
pub const OPTIFINE: &str = "http://s.optifine.net";
/// The most a cape picture may weigh: real ones are a few kilobytes.
const MAX_PICTURE: u64 = 256 * 1024;

/// One cape the account owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cape {
    pub id: String,
    /// Its name ("Migrator").
    pub alias: String,
    /// Its picture on textures.minecraft.net.
    pub url: String,
    /// The account wears it.
    pub active: bool,
}

/// The capes a profile (`/minecraft/profile`) lists, in its order.
pub fn owned(profile: &Value) -> Vec<Cape> {
    profile
        .get("capes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Cape {
                id: str_field(c, "id")?,
                alias: str_field(c, "alias").unwrap_or_default(),
                url: str_field(c, "url").filter(|u| is_texture(u))?,
                active: c.get("state").and_then(Value::as_str) == Some("ACTIVE"),
            })
        })
        .collect()
}

/// A picture on Mojang's texture server, the only place a Minecraft cape's picture is fetched
/// from (as the game, after authlib, does).
pub fn is_texture(url: &str) -> bool {
    [
        "http://textures.minecraft.net/texture/",
        "https://textures.minecraft.net/texture/",
    ]
    .iter()
    .any(|base| {
        url.strip_prefix(base)
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
    })
}

/// The capes the account with `token` owns.
pub fn list(ep: &Endpoints, token: &str) -> Result<Vec<Cape>, String> {
    let (status, v) = send(
        bearer(
            request_to("GET", &format!("{}/minecraft/profile", ep.minecraft)),
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    answer(status, &v)
}

/// Wears the cape `id`, or none; the capes as they are after it.
pub fn wear(ep: &Endpoints, token: &str, id: Option<&str>) -> Result<Vec<Cape>, String> {
    let url = format!("{}/minecraft/profile/capes/active", ep.minecraft);
    let (status, v) = match id {
        Some(id) => send(
            bearer(request_to("PUT", &url), token),
            Some(Body::Json(json!({ "capeId": id }))),
            "the Minecraft services",
        )?,
        None => send(
            bearer(request_to("DELETE", &url), token),
            None,
            "the Minecraft services",
        )?,
    };
    answer(status, &v)
}

fn answer(status: u16, v: &Value) -> Result<Vec<Cape>, String> {
    match status {
        200 => Ok(owned(v)),
        401 => Err(super::SIGN_IN_AGAIN.into()),
        400 | 404 => Err("This account doesn't own that cape.".into()),
        429 => Err("Too many cape changes in a short time. Wait a minute, then try again.".into()),
        _ => Err(format!(
            "The Minecraft services couldn't change the cape (error {status}). Try again later."
        )),
    }
}

/// A name OptiFine's cape server can have a cape under: a Minecraft name, 1 to 16 letters,
/// digits and underscores. Nothing else is ever put in its URL.
pub fn optifine_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The OptiFine cape of `name` on the cape server at `base` ([`OPTIFINE`]): its PNG, or none when
/// the player has none.
pub fn optifine(base: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
    if !optifine_name(name) {
        return Ok(None);
    }
    let url = format!("{base}/capes/{name}.png");
    let agent = crate::proxy::agent(
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .user_agent(&format!("sunrise-launcher/{}", crate::backend::VERSION)),
        &url,
    )
    .build();
    let response = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(ureq::Error::Status(status, _)) => {
            return Err(format!(
                "OptiFine's cape server answered with error {status}."
            ))
        }
        Err(ureq::Error::Transport(_)) => {
            return Err("Could not reach OptiFine's cape server.".into())
        }
    };
    let bytes = read_picture(response.into_reader())
        .ok_or("OptiFine's cape server sent something that isn't a cape picture.")?;
    Ok(Some(bytes))
}

/// A skin's or cape's picture from [`is_texture`]'s server; none when it can't be had.
pub fn picture(url: &str) -> Option<Vec<u8>> {
    if !is_texture(url) {
        return None;
    }
    let response = crate::proxy::agent(
        ureq::AgentBuilder::new().timeout(Duration::from_secs(10)),
        url,
    )
    .build()
    .get(url)
    .call()
    .ok()?;
    read_picture(response.into_reader())
}

/// At most [`MAX_PICTURE`] bytes that decode as a PNG.
fn read_picture(reader: impl Read) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_PICTURE + 1).read_to_end(&mut bytes).ok()?;
    let fits = bytes.len() as u64 <= MAX_PICTURE;
    (fits && image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).is_ok())
        .then_some(bytes)
}
